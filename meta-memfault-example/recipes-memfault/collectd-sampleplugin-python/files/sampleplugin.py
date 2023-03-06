#
# Copyright 2023 Memfault, Inc
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
import collectd

path = "/proc/uptime"


def config_fn(config):
    for node in config.children:
        key = node.key.lower()
        if key == "path":
            global path
            path = node.values[0]
        else:
            collectd.info('sampleplugin: unrecognised option "%s"' % (key))


def read_fn():
    with open(path, "rb") as f:
        uptime = float(f.readline().split()[0])

    val = collectd.Values(type="uptime")
    val.plugin = "myuptime"
    val.dispatch(values=[uptime])


collectd.register_config(config_fn)
collectd.register_read(read_fn)
