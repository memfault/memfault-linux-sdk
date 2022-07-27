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
#include <stdlib.h>
#include <string.h>

#include "memfaultd.h"

struct MemfaultdNetwork {
  sMemfaultd *memfaultd;
  CURL *curl;
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

/**
 * @brief Initialises the network object
 *
 * @param memfaultd Main memfaultd handle
 * @return memfaultd_network_h network object
 */
sMemfaultdNetwork *memfaultd_network_init(sMemfaultd *memfaultd) {
  sMemfaultdNetwork *handle = calloc(sizeof(sMemfaultdNetwork), 1);

  handle->memfaultd = memfaultd;

  if (!(handle->curl = curl_easy_init())) {
    fprintf(stderr, "network:: Failed to initialise CURL.\n");
    free(handle);
    return NULL;
  }

  return handle;
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
    free(handle);
  }
}

/**
 * @brief Perform GET against a given endpoint
 *
 * @param handle network object
 * @param endpoint Path
 * @param data Data returned
 * @param len Length of data returned
 * @return true Successfully performed the GET
 * @return false Failed to perform
 */
bool memfaultd_network_get(sMemfaultdNetwork *handle, const char *endpoint, char **data,
                           size_t *len) {
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  const char *base_url;
  if (!memfaultd_get_string(handle->memfaultd, "", "base_url", &base_url)) {
    return false;
  }

  char *url = malloc(strlen(endpoint) + strlen(base_url) + 1);
  strcpy(url, base_url);
  strcat(url, endpoint);

  struct _write_callback recv_buf;
  recv_buf.buf = malloc(1);
  recv_buf.size = 0;

  curl_easy_setopt(handle->curl, CURLOPT_URL, url);
  curl_easy_setopt(handle->curl, CURLOPT_NOPROGRESS, 1L);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEFUNCTION, prv_network_write_callback);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEDATA, (void *)&recv_buf);

  CURLcode res = curl_easy_perform(handle->curl);
  curl_easy_reset(handle->curl);

  if (res != CURLE_OK) {
    fprintf(stderr, "network:: Failed to GET message from %s, %u, %s.\n", url, res,
            curl_easy_strerror(res));
    free(url);
    free(recv_buf.buf);
    return false;
  }

  free(url);

  *len = recv_buf.size;
  *data = recv_buf.buf;
  return true;
}

/**
 * @brief Perform POST against a given endpoint
 *
 * @param handle network object
 * @param endpoint Path
 * @param payload Data to send
 * @param data Data returned if available
 * @param len Length of data returned
 * @return true Successfully performed the POST
 * @return false Failed to perform
 */
bool memfaultd_network_post(sMemfaultdNetwork *handle, const char *endpoint, const char *payload,
                            char **data, size_t *len) {
  const sMemfaultdDeviceSettings *settings = memfaultd_get_device_settings(handle->memfaultd);

  const char *base_url;
  if (!memfaultd_get_string(handle->memfaultd, "", "base_url", &base_url)) {
    return false;
  }

  char *url = malloc(strlen(endpoint) + strlen(base_url) + 1);
  strcpy(url, base_url);
  strcat(url, endpoint);

  struct _write_callback recv_buf = {0};
  if (data) {
    recv_buf.buf = malloc(1);
    recv_buf.size = 0;
  }

  struct curl_slist *headers = NULL;
  headers = curl_slist_append(headers, "Accept: application/json");
  headers = curl_slist_append(headers, "Content-Type: application/json");
  headers = curl_slist_append(headers, "charset: utf-8");

  const char *project_key = "";
  if (memfaultd_get_string(handle->memfaultd, "", "project_key", &project_key)) {
    char project_key_header[256];
    snprintf(project_key_header, sizeof(project_key_header), "Memfault-Project-Key: %s",
             project_key);
    headers = curl_slist_append(headers, project_key_header);
  }

  curl_easy_setopt(handle->curl, CURLOPT_URL, url);
  curl_easy_setopt(handle->curl, CURLOPT_POSTFIELDS, payload);
  curl_easy_setopt(handle->curl, CURLOPT_HTTPHEADER, headers);
  curl_easy_setopt(handle->curl, CURLOPT_NOPROGRESS, 1L);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEFUNCTION, prv_network_write_callback);
  curl_easy_setopt(handle->curl, CURLOPT_WRITEDATA, (void *)&recv_buf);
  CURLcode res = curl_easy_perform(handle->curl);
  curl_easy_reset(handle->curl);
  curl_slist_free_all(headers);

  if (res != CURLE_OK) {
    fprintf(stderr, "network:: Failed to POST message to %s, %u, %s.\n", url, res,
            curl_easy_strerror(res));
    free(url);
    if (data) {
      free(recv_buf.buf);
    }
    return false;
  }

  free(url);

  if (data) {
    *len = recv_buf.size;
    *data = recv_buf.buf;
  }

  return true;
}
