#pragma once

//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//! memfaultctl

#ifdef __cplusplus
extern "C" {
#endif

int cmd_disable_data_collection(char *config_file);
int cmd_disable_developer_mode(char *config_file);
int cmd_enable_data_collection(char *config_file);
int cmd_enable_developer_mode(char *config_file);
int cmd_reboot(char *config_file, int reboot_reason);
int cmd_request_metrics(void);

#ifdef __cplusplus
}
#endif
