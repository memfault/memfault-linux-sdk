cmake_minimum_required(VERSION 3.18)
include(CppUTest.cmake)

set(SRC_DIR ../src)
set(PLUGINS_DIR ../src/plugins)

function(add_memfaultd_cpputest_target NAME)
    add_cpputest_target(${NAME} ${ARGN})

    if (NOT MACOS_BUILD)
        # On Linux we want to let the linker select a compatible (32/64) version of the lib
        target_link_libraries(${NAME} ${JSONC_LIBRARIES})
    else()
        # On Mac we need the complete import which will include the exact path to the file
        target_link_libraries(${NAME} PkgConfig::JSONC PkgConfig::ZLIB)
    endif()
    target_include_directories(${NAME} PRIVATE ${SRC_DIR})
    target_include_directories(${NAME} PRIVATE ${PLUGINS_DIR})
    target_include_directories(${NAME} PUBLIC ${JSONC_INCLUDE_DIRS} ${UUID_INCLUDE_DIRS})
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
target_compile_definitions(test_reboot PUBLIC HAVE_SYSTEMD)
target_link_libraries(test_reboot dl)
if (MACOS_BUILD)
    target_include_directories(test_reboot SYSTEM PRIVATE macos-stubs)
    target_sources(test_reboot PRIVATE macos-stubs/systemd/systemd-stub.c)
endif()

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

# Coredump code does not compile (yet) on macOS because it
# lacks elf.h
if(NOT MACOS_BUILD)
add_memfaultd_cpputest_target(test_core_elf_metadata
    core_elf_metadata.test.cpp
    ${SRC_DIR}/memfault-core-handler/core_elf_metadata.c
    ${SRC_DIR}/memfault-core-handler/core_elf_note.c
    ${SRC_DIR}/util/cbor.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_core_elf_note
    core_elf_note.test.cpp
    ${SRC_DIR}/memfault-core-handler/core_elf_note.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_core_elf_reader
    core_elf_reader.test.cpp
    ${SRC_DIR}/memfault-core-handler/core_elf_reader.c
    ${SRC_DIR}/util/string.c
    core_elf_memory_io.c
)

add_memfaultd_cpputest_target(test_core_elf_writer
    core_elf_writer.test.cpp
    ${SRC_DIR}/memfault-core-handler/core_elf_writer.c
    core_elf_memory_io.c
)
target_link_libraries(test_core_elf_writer ${ZLIB_LIBRARIES})

add_memfaultd_cpputest_target(test_core_elf_transformer
    core_elf_transformer.test.cpp
    ${SRC_DIR}/memfault-core-handler/core_elf_metadata.c
    ${SRC_DIR}/memfault-core-handler/core_elf_note.c
    ${SRC_DIR}/memfault-core-handler/core_elf_reader.c
    ${SRC_DIR}/memfault-core-handler/core_elf_transformer.c
    ${SRC_DIR}/memfault-core-handler/core_elf_writer.c
    ${SRC_DIR}/util/cbor.c
    ${SRC_DIR}/util/string.c
    core_elf_memory_io.c
)
target_link_libraries(test_core_elf_transformer ${ZLIB_LIBRARIES})

add_memfaultd_cpputest_target(test_coredump_ratelimiter
    coredump_ratelimiter.test.cpp
    ${SRC_DIR}/memfault-core-handler/coredump_ratelimiter.c
)
endif()

add_memfaultd_cpputest_target(test_rate_limiter
    rate_limiter.test.cpp
    ${SRC_DIR}/util/rate_limiter.c
)
