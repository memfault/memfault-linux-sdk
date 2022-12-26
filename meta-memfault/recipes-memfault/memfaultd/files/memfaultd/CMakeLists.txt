cmake_minimum_required(VERSION 3.16)

project(memfaultd)

option(TESTS "Enable unit tests" ON)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

include(FindPkgConfig)

pkg_check_modules(SDBUS REQUIRED libsystemd)
pkg_check_modules(CURL REQUIRED libcurl)
pkg_check_modules(JSON-C REQUIRED json-c)
pkg_check_modules(UBOOTENV REQUIRED libubootenv)
pkg_check_modules(UUID REQUIRED uuid)
pkg_check_modules(ZLIB REQUIRED zlib)

include_directories(include ${CMAKE_CURRENT_BINARY_DIR})

add_custom_command(
    OUTPUT ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    COMMAND xxd -i builtin.conf > ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    DEPENDS builtin.conf
)

list(APPEND sources
    ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    src/main.c
    src/memfaultctl/crash.c
    src/memfaultctl/memfaultctl.c
    src/memfaultctl/parse_attributes.c
    src/memfaultd.c
    src/network.c
    src/queue.c
    src/plugins/attributes/attributes.c
    src/util/cbor.c
    src/util/config.c
    src/util/device_settings.c
    src/util/disk.c
    src/util/dump_settings.c
    src/util/ipc.c
    src/util/linux_boot_id.c
    src/util/pid.c
    src/util/plugins.c
    src/util/runtime_config.c
    src/util/rate_limiter.c
    src/util/reboot_reason.c
    src/util/string.c
    src/util/systemd.c
    src/util/version.c
)

if(PLUGIN_REBOOT)
    list(APPEND sources
        src/plugins/reboot/reboot.c
        src/plugins/reboot/reboot_last_boot_id.c
        src/plugins/reboot/reboot_process_pstore.c
    )
    add_definitions("-DPLUGIN_REBOOT")

    find_package(libuboot)

    list(APPEND plugin_libraries ${libuboot_LIBRARIES})
endif()

if(PLUGIN_SWUPDATE)
    list(APPEND sources src/plugins/swupdate.c)
    add_definitions("-DPLUGIN_SWUPDATE")

    include(FindPkgConfig)
    pkg_check_modules(LIBCONFIG REQUIRED libconfig)

    list(APPEND plugin_libraries ${LIBCONFIG_LIBRARIES})
endif()

if(PLUGIN_COLLECTD)
    list(APPEND sources src/plugins/collectd/collectd.c)
    add_definitions("-DPLUGIN_COLLECTD")
endif()

if(PLUGIN_COREDUMP)
    list(APPEND sources
       src/plugins/coredump/coredump.c
       src/plugins/coredump/core_elf_metadata.c
       src/plugins/coredump/core_elf_note.c
       src/plugins/coredump/core_elf_reader.c
       src/plugins/coredump/core_elf_transformer.c
       src/plugins/coredump/core_elf_writer.c
       src/plugins/coredump/coredump_ratelimiter.c
    )
    add_definitions("-DPLUGIN_COREDUMP")

    include(FindPkgConfig)
    pkg_check_modules(LIBUUID REQUIRED uuid)

    list(APPEND plugin_libraries ${LIBUUID_LIBRARIES} ${ZLIB_LIBRARIES})
endif()

add_executable(memfaultd ${sources})

add_definitions("-D_DEFAULT_SOURCE=1")

target_link_libraries(memfaultd PUBLIC ${CURL_LIBRARIES} ${SDBUS_LIBRARIES} ${JSON-C_LIBRARIES} pthread ${plugin_libraries})
target_compile_options(memfaultd PRIVATE
    -O3
    -g3
    -Wall
    -Wpedantic
    -Wextra
    -Werror
    -Wno-unused-parameter
    -std=c11
)

ADD_CUSTOM_TARGET(memfaultctl ALL COMMAND ${CMAKE_COMMAND} -E create_symlink memfaultd memfaultctl)

install(TARGETS memfaultd RUNTIME)
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/memfaultctl DESTINATION bin)
install(FILES ${CMAKE_CURRENT_SOURCE_DIR}/memfaultd.conf TYPE SYSCONF)

if(TESTS)
    enable_testing()
    add_subdirectory(test)
endif()
