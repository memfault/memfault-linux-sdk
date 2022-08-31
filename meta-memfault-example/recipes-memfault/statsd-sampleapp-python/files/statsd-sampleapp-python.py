#
# Copyright 2022 Memfault, Inc
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
#!/usr/bin/env python3

import time
from random import random

from statsd import StatsClient

statsd = StatsClient(
    host="localhost", port=8125, prefix="mypythonapp", maxudpsize=512, ipv6=False
)

while True:
    statsd.incr("mycount", 3 * random())
    statsd.gauge("mygauge", 100 * random())
    with statsd.timer("mytime"):
        time.sleep(2 * random())
