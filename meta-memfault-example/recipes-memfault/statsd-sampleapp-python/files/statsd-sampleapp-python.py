#!/usr/bin/env python3
#
# Copyright 2024 Memfault, Inc
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import time
from random import random

from statsd import StatsClient

statsd = StatsClient(host="localhost", port=8125, prefix="mypythonapp", maxudpsize=512, ipv6=False)

while True:
    statsd.incr("mycount", int(3 * random()))  # noqa: S311
    statsd.gauge("mygauge", int(100 * random()))  # noqa: S311
    with statsd.timer("mytime"):
        time.sleep(2 * random())  # noqa: S311
