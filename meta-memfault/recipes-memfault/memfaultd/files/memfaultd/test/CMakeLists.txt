cmake_minimum_required(VERSION 3.18)
include(CppUTest.cmake)

set(SRC_DIR ../src)
set(PLUGINS_DIR ../src/plugins)

pkg_check_modules(SDBUS REQUIRED libsystemd)
pkg_check_modules(ZLIB REQUIRED zlib)

function(add_memfaultd_cpputest_target NAME)
    add_cpputest_target(${NAME} ${ARGN})

    target_link_libraries(${NAME} ${CURL_LIBRARIES} ${JSON-C_LIBRARIES})
    target_include_directories(${NAME} PRIVATE ${SRC_DIR})
    target_include_directories(${NAME} PRIVATE ${PLUGINS_DIR})
endfunction()

#### TEST TARGET DEFINITIONS ####

add_memfaultd_cpputest_target(test_queue
    queue.test.cpp
    ${SRC_DIR}/queue.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_device_settings
    device_settings.test.cpp
    ${SRC_DIR}/util/device_settings.c
)

add_memfaultd_cpputest_target(test_reboot
    reboot.test.cpp
    ${PLUGINS_DIR}/reboot/reboot.c
    ${SRC_DIR}/util/reboot_reason.c
)
target_link_options(test_reboot PRIVATE -Wl,--wrap=access)

add_memfaultd_cpputest_target(test_reboot_last_boot_id
    reboot_last_boot_id.test.cpp
    ${PLUGINS_DIR}/reboot/reboot_last_boot_id.c
)

add_memfaultd_cpputest_target(test_reboot_process_pstore
    reboot_process_pstore.test.cpp
    ${PLUGINS_DIR}/reboot/reboot_process_pstore.c
)

add_memfaultd_cpputest_target(test_collectd
    collectd.test.cpp
    ${PLUGINS_DIR}/collectd/collectd.c
    ${SRC_DIR}/util/string.c
)
target_link_libraries(test_collectd ${SDBUS_LIBRARIES})

add_memfaultd_cpputest_target(test_cbor
    cbor.test.cpp
    ${SRC_DIR}/util/cbor.c
)

add_memfaultd_cpputest_target(test_core_elf_metadata
    core_elf_metadata.test.cpp
    ${PLUGINS_DIR}/coredump/core_elf_metadata.c
    ${PLUGINS_DIR}/coredump/core_elf_note.c
    ${SRC_DIR}/util/cbor.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_core_elf_note
    core_elf_note.test.cpp
    ${PLUGINS_DIR}/coredump/core_elf_note.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_core_elf_reader
    core_elf_reader.test.cpp
    ${PLUGINS_DIR}/coredump/core_elf_reader.c
    ${SRC_DIR}/util/string.c
    core_elf_memory_io.c
)

add_memfaultd_cpputest_target(test_core_elf_writer
    core_elf_writer.test.cpp
    ${PLUGINS_DIR}/coredump/core_elf_writer.c
    core_elf_memory_io.c
)
target_link_libraries(test_core_elf_writer ${ZLIB_LIBRARIES})

add_memfaultd_cpputest_target(test_core_elf_transformer
    core_elf_transformer.test.cpp
    ${PLUGINS_DIR}/coredump/core_elf_metadata.c
    ${PLUGINS_DIR}/coredump/core_elf_note.c
    ${PLUGINS_DIR}/coredump/core_elf_reader.c
    ${PLUGINS_DIR}/coredump/core_elf_transformer.c
    ${PLUGINS_DIR}/coredump/core_elf_writer.c
    ${SRC_DIR}/util/cbor.c
    ${SRC_DIR}/util/string.c
    core_elf_memory_io.c
)
target_link_libraries(test_core_elf_transformer ${ZLIB_LIBRARIES})

add_memfaultd_cpputest_target(test_coredump_ratelimiter
    coredump_ratelimiter.test.cpp
    ${PLUGINS_DIR}/coredump/coredump_ratelimiter.c
)

add_memfaultd_cpputest_target(test_rate_limiter
    rate_limiter.test.cpp
    ${SRC_DIR}/util/rate_limiter.c
)

add_memfaultd_cpputest_target(test_parse_attributes
    parse_attributes.test.cpp
    ${SRC_DIR}/memfaultctl/parse_attributes.c)
