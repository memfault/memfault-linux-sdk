//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! Network POST & GET API wrapper around libCURL
//!

#include "network.h"

#include <curl/curl.h>
#include <errno.h>
#include <fcntl.h>
#include <json-c/json.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "memfault/util/string.h"
#include "memfaultd.h"

struct MemfaultdNetwork {
  sMemfaultd *memfaultd;
  bool during_network_failure;
  CURL *curl;
  const char *base_url;
  char *project_key_header;
  const char *software_type;
  const char *software_version;
};

struct _write_callback {
  char *buf;
  size_t size;
};

/**
 * @brief libCURL write callback
 *
 * @param contents Data received
 * @param size Size of buffer received
 * @param nmemb Number of elements in buffer
 * @param userp Write callback context
 * @return size_t Bytes processed
 */
static size_t prv_network_write_callback(void *contents, size_t size, size_t nmemb, void *userp) {
  struct _write_callback *recv_buf = userp;
  size_t total_size = (size * nmemb);

  if (!recv_buf->buf) {
    return total_size;
  }

  char *ptr = realloc(recv_buf->buf, recv_buf->size + total_size + 1);
  if (!ptr) {
    fprintf(stderr, "network:: Failed to allocate memory for network GET.\n");
    return 0;
  }

  recv_buf->buf = ptr;
  memcpy(&(recv_buf->buf[recv_buf->size]), contents, total_size);
  recv_buf->size += total_size;
  recv_buf->buf[recv_buf->size] = 0;

  return total_size;
}

static void prv_log_first_failed_request(sMemfaultdNetwork *handle, const char *fmt, ...) {
  if (!handle->during_network_failure) {
    va_list args;

    va_start(args, fmt);
    vfprintf(stderr, fmt, args);
    va_end(args);

    handle->during_network_failure = true;
  }
}

static void prv_log_first_succeeded_request(sMemfaultdNetwork *handle, const char *fmt, ...) {
  if (handle->during_network_failure) {
    va_list args;

    va_start(args, fmt);
    vfprintf(stderr, fmt, args);
    va_end(args);

    handle->during_network_failure = false;
  }
}

static eMemfaultdNetworkResult prv_check_error(sMemfaultdNetwork *handle, const CURLcode res,
                                               const char *method, const char *url) {
  long http_code = 0;
  curl_easy_getinfo(handle->curl, CURLINFO_HTTP_CODE, &http_code);

  // Note: assuming NOT using CURLOPT_FAILONERROR!
  if (res != CURLE_OK) {
    prv_log_first_failed_request(
      handle, "network:: Failed to perform %s request to %s, %d: %s (HTTP code %ld).\n", method,
      url, res, curl_easy_strerror(res), http_code);
    return kMemfaultdNetworkResult_ErrorRetryLater;
  }

  prv_log_first_succeeded_request(
    handle,
    "network:: Network recovered, successfully performed %s request to %s (HTTP code %ld).\n",
    method, url, http_code);

  if (http_code >= 400 && http_code <= 499) {
    // Client error:
    fprintf(stderr, "network:: client error for %s request to %s (HTTP code %ld).\n", method, url,
            http_code);
    return kMemfaultdNetworkResult_ErrorNoRetry;
  } else if (http_code >= 500 && http_code <= 599) {
    // Server error:
    fprintf(stderr, "network:: server error for %s request to %s (HTTP code %ld).\n", method, url,
            http_code);
    return kMemfaultdNetworkResult_ErrorRetryLater;
  }
  return kMemfaultdNetworkResult_OK;
}

static char *prv_create_url(sMemfaultdNetwork *handle, const char *endpoint) {
  char *url;
  if (memfault_asprintf(&url, "%s%s", handle->base_url, endpoint) == -1) {
    return NULL;
  }
  return url;
}

static bool prv_parse_file_upload_prepare_response(const char *recvdata, char **upload_url,
                                                   char **upload_token) {
  json_object *payload_object = NULL;
  *upload_url = NULL;
  *upload_token = NULL;

  if (!(payload_object = json_tokener_parse(recvdata))) {
    fprintf(stderr, "network:: Failed to parse file upload request response\n");
    goto cleanup;
  }

  json_object *data_object;
  if (!json_object_object_get_ex(payload_object, "data", &data_object) ||
      json_object_get_type(data_object) != json_type_object) {
    fprintf(stderr, "network:: File upload request response missing 'data'\n");
    goto cleanup;
  }

  json_object *object;
  if (!json_object_object_get_ex(data_object, "upload_url", &object) ||
      json_object_get_type(object) != json_type_string) {
    fprintf(stderr, "network:: File upload request response missing 'upload_url'\n");
    goto cleanup;
  }
  *upload_url = strdup(json_object_get_string(object));

  if (!json_object_object_get_ex(data_object, "token", &object) ||
      json_object_get_type(object) != json_type_string) {
    fprintf(stderr, "network:: File upload request response missing 'token'\n");
    goto cleanup;
  }
  *upload_token = strdup(json_object_get_string(object));

  json_object_put(payload_object);
  return true;

cleanup:
  free(*upload_url);
  free(*upload_token);
  json_object_put(payload_object);
  return false;
}

static eMemfaultdNetworkResult prv_file_upload_prepare(sMemfaultdNetwork *handle,
                                                       const char *endpoint, const size_t filesize,
                                                       char **upload_url, char **upload_token) {
  char *recvdata = NULL;
  size_t recvlen;
  eMemfaultdNetworkResult rc;
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  char *upload_request = NULL;
  char *upload_request_fmt = "{"
                             "  \"kind\": \"ELF_COREDUMP\","
                             "  \"device\": {"
                             "    \"device_serial\": \"%s\","
                             "    \"hardware_version\": \"%s\","
                             "    \"software_version\": \"%s\","
                             "    \"software_type\": \"%s\""
                             "  },"
                             "  \"size\": %d"
                             "}";
  if (memfault_asprintf(&upload_request, upload_request_fmt, settings->device_id,
                        settings->hardware_version, handle->software_version, handle->software_type,
                        filesize) == -1) {
    rc = kMemfaultdNetworkResult_ErrorRetryLater;
    goto cleanup;
  }

  rc = memfaultd_network_post(handle, endpoint, upload_request, &recvdata, &recvlen);
  if (rc != kMemfaultdNetworkResult_OK) {
    goto cleanup;
  }
  recvdata[recvlen - 1] = '\0';

  if (!prv_parse_file_upload_prepare_response(recvdata, upload_url, upload_token)) {
    rc = kMemfaultdNetworkResult_ErrorRetryLater;
    goto cleanup;
  }

  rc = kMemfaultdNetworkResult_OK;

cleanup:
  free(upload_request);
  free(recvdata);
  return rc;
}

static eMemfaultdNetworkResult prv_file_upload(sMemfaultdNetwork *handle, const char *url,
                                               const char *filename, const size_t filesize) {
  eMemfaultdNetworkResult rc;

  FILE *fd;
  if (!(fd = fopen(filename, "rb"))) {
    fprintf(stderr, "network:: Failed to open upload file %s : %s", filename, strerror(errno));
    rc = kMemfaultdNetworkResult_ErrorNoRetry;
    goto cleanup;
  }

  curl_easy_setopt(handle->curl, CURLOPT_URL, url);
  curl_easy_setopt(handle->curl, CURLOPT_UPLOAD, 1L);
  curl_easy_setopt(handle->curl, CURLOPT_READDATA, fd);
  curl_easy_setopt(handle->curl, CURLOPT_INFILESIZE_LARGE, (curl_off_t)filesize);

  const CURLcode res = curl_easy_perform(handle->curl);
  rc = prv_check_error(handle, res, "PUT", url);

cleanup:
  curl_easy_reset(handle->curl);
  fclose(fd);
  return rc;
}

static eMemfaultdNetworkResult prv_file_upload_commit(sMemfaultdNetwork *handle,
                                                      const char *endpoint, const char *token) {
  eMemfaultdNetworkResult rc;
  char *payload = NULL;
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  const char *payload_fmt = "{"
                            "  \"file\": {"
                            "    \"token\": \"%s\""
                            "  },"
                            "  \"device\": {"
                            "    \"device_serial\": \"%s\","
                            "    \"hardware_version\": \"%s\","
                            "    \"software_version\": \"%s\","
                            "    \"software_type\": \"%s\""
                            "  }"
                            "}";
  if (memfault_asprintf(&payload, payload_fmt, token, settings->device_id,
                        settings->hardware_version, handle->software_version,
                        handle->software_type) == -1) {
    rc = kMemfaultdNetworkResult_ErrorRetryLater;
    goto cleanup;
  }

  rc = memfaultd_network_post(handle, endpoint, payload, NULL, 0);

cleanup:
  free(payload);

  return rc;
}

/**
 * @brief Initialises the network object
 *
 * @param memfaultd Main memfaultd handle
 * @return memfaultd_network_h network object
 */
sMemfaultdNetwork *memfaultd_network_init(sMemfaultd *memfaultd) {
  sMemfaultdNetwork *handle = calloc(sizeof(sMemfaultdNetwork), 1);
  if (!handle) {
    fprintf(stderr, "network:: Failed to allocate memory for handle\n");
    goto cleanup;
  }

  handle->memfaultd = memfaultd;
  handle->during_network_failure = false;

  if (!(handle->curl = curl_easy_init())) {
    fprintf(stderr, "network:: Failed to initialise CURL.\n");
    goto cleanup;
  }

  if (!memfaultd_get_string(handle->memfaultd, "", "software_type", &handle->software_type) ||
      strlen(handle->software_type) == 0) {
    fprintf(stderr, "network:: Failed to get software_type\n");
    goto cleanup;
  }

  if (!memfaultd_get_string(handle->memfaultd, "", "software_version", &handle->software_version) ||
      strlen(handle->software_version) == 0) {
    fprintf(stderr, "network:: Failed to get software_version\n");
    goto cleanup;
  }

  if (!memfaultd_get_string(handle->memfaultd, "", "base_url", &handle->base_url) ||
      strlen(handle->base_url) == 0) {
    fprintf(stderr, "network:: Failed to get base_url\n");
    goto cleanup;
  }

  const char *project_key;
  if (!memfaultd_get_string(handle->memfaultd, "", "project_key", &project_key) ||
      strlen(project_key) == 0) {
    fprintf(stderr, "network:: Failed to get project_key\n");
    goto cleanup;
  }

  char *project_key_fmt = "Memfault-Project-Key: %s";
  if (memfault_asprintf(&handle->project_key_header, project_key_fmt, project_key) == -1) {
    goto cleanup;
  }

  return handle;

cleanup:
  free(handle->project_key_header);
  free(handle);
  return NULL;
}

/**
 * @brief Destroy the network object
 *
 * @param handle network object
 */
void memfaultd_network_destroy(sMemfaultdNetwork *handle) {
  if (handle) {
    if (handle->curl) {
      curl_easy_cleanup(handle->curl);
    }
    free(handle->project_key_header);
    free(handle);
  }
}

/**
 * @brief Perform POST against a given endpoint
 *
 * @param handle network object
 * @param endpoint Path
 * @param payload Data to send
 * @param data Data returned if available
 * @param len Length of data returned
 * @return A eMemfaultdNetworkResult value indicating whether the POST was successful or not.
 */
eMemfaultdNetworkResult memfaultd_network_post(sMemfaultdNetwork *handle, const char *endpoint,
                                               const char *payload, char **data, size_t *len) {
  char *url = prv_create_url(handle, endpoint);
  if (!url) {
    return kMemfaultdNetworkResult_ErrorRetryLater;
  }

  struct _write_callback recv_buf = {0};
  if (data) {
    recv_buf.buf = malloc(1);
    recv_buf.size = 0;
  }

  struct curl_slist *headers = NULL;
  headers = curl_slist_append(headers, "Accept: application/json");
  headers = curl_slist_append(headers, "Content-Type: application/json");
  headers = curl_slist_append(headers, "charset: utf-8");
  headers = curl_slist_append(headers, handle->project_key_header);

  curl_easy_setopt(handle->curl, CURLOPT_URL, url);
  curl_easy_setopt(handle->curl, CURLOPT_POSTFIELDS, payload);
  curl_easy_setopt(handle->curl, CURLOPT_HTTPHEADER, headers);
  curl_easy_setopt(handle->curl, CURLOPT_NOPROGRESS, 1L);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEFUNCTION, prv_network_write_callback);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEDATA, (void *)&recv_buf);
  const CURLcode res = curl_easy_perform(handle->curl);
  curl_slist_free_all(headers);

  const eMemfaultdNetworkResult result = prv_check_error(handle, res, "POST", url);

  free(url);

  if (result == kMemfaultdNetworkResult_OK) {
    if (data) {
      *len = recv_buf.size;
      *data = recv_buf.buf;
    }
  } else {
    if (data) {
      free(recv_buf.buf);
    }
  }

  curl_easy_reset(handle->curl);
  return result;
}

eMemfaultdNetworkResult memfaultd_network_file_upload(sMemfaultdNetwork *handle,
                                                      const char *commit_endpoint,
                                                      const char *filename) {
  eMemfaultdNetworkResult rc;
  char *upload_url = NULL;
  char *upload_token = NULL;

  struct stat st;
  if (stat(filename, &st) == -1) {
    fprintf(stderr, "network:: Failed to stat file '%s' : %s\n", filename, strerror(errno));
    rc = kMemfaultdNetworkResult_ErrorNoRetry;
    goto cleanup;
  }

  rc = prv_file_upload_prepare(handle, "/api/v0/upload", st.st_size, &upload_url, &upload_token);
  if (rc != kMemfaultdNetworkResult_OK) {
    goto cleanup;
  }

  rc = prv_file_upload(handle, upload_url, filename, st.st_size);
  if (rc != kMemfaultdNetworkResult_OK) {
    goto cleanup;
  }

  rc = prv_file_upload_commit(handle, commit_endpoint, upload_token);
  if (rc != kMemfaultdNetworkResult_OK) {
    goto cleanup;
  }

  fprintf(stderr, "network:: Successfully transmitted file '%s'\n", filename);

  unlink(filename);
  rc = kMemfaultdNetworkResult_OK;

cleanup:
  free(upload_url);
  free(upload_token);
  return rc;
}
