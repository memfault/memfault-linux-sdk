//! @file
//!
//! Copyright 2022 Memfault, Inc
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//!     http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.
#include <statsd-client.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <unistd.h>

#define MAX_LINE_LEN 200
#define PKT_LEN 1400

int main(int argc, char *argv[]) {
  statsd_link *link;

  link = statsd_init_with_namespace("localhost", 8125, "mycapp");

  while (1) {
    char pkt[PKT_LEN] = {'\0'};
    char tmp[MAX_LINE_LEN] = {'\0'};
    struct timeval start, end;
    int ms;

    statsd_prepare(link, "mycount", rand() % 3, "c", 1.0, tmp, MAX_LINE_LEN, 1);
    strncat(pkt, tmp, PKT_LEN - 1);

    statsd_prepare(link, "mygauge", rand() % 100, "g", 1.0, tmp, MAX_LINE_LEN,
                   1);
    strncat(pkt, tmp, PKT_LEN - 1);

    gettimeofday(&start, NULL);
    sleep(rand() % 2);
    gettimeofday(&end, NULL);
    ms = ((end.tv_sec - start.tv_sec) * 1000) +
         ((end.tv_usec - start.tv_usec) / 1000);

    statsd_prepare(link, "mytime", ms, "ms", 1.0, tmp, MAX_LINE_LEN, 1);
    strncat(pkt, tmp, PKT_LEN - 1);

    statsd_send(link, pkt);
  }

  statsd_finalize(link);

  return 0;
}
