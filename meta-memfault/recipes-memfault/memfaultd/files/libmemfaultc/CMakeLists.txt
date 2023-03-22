cmake_minimum_required(VERSION 3.16)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_EXTENSIONS ON)
set(CMAKE_CXX_STANDARD 11)

project(memfaultd)

option(TESTS "Enable unit tests" ON)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

find_package(PkgConfig REQUIRED)

pkg_check_modules(JSONC REQUIRED IMPORTED_TARGET json-c)
pkg_check_modules(ZLIB REQUIRED IMPORTED_TARGET zlib)

# We have to disable some features to build on mac.
if (${CMAKE_SYSTEM_NAME} MATCHES "Darwin")
    set(MACOS_BUILD 1)
endif()

if (NOT MACOS_BUILD)
    pkg_check_modules(UUID REQUIRED uuid)
    pkg_check_modules(SDBUS REQUIRED libsystemd)
    add_definitions("-DHAVE_SYSTEMD")
else()
    # You can install libuuid1 on macOS via `brew install util-linux`
    # Note that macOS built-in UUID and brew ossp-uuid are not the same thing as libuuid1.
    execute_process(COMMAND brew --prefix OUTPUT_VARIABLE BREW_PREFIX OUTPUT_STRIP_TRAILING_WHITESPACE)

    if(NOT EXISTS "${BREW_PREFIX}/Cellar/util-linux/2.38.1")
        message(FATAL_ERROR "Package 'util-linux' not found. You need to `brew install util-linux` to build memfaultd on macOS. BREW_PREFIX=${BREW_PREFIX}XX")
    endif()
    set(UUID_INCLUDE_DIRS "${BREW_PREFIX}/Cellar/util-linux/2.38.1/include/")
    set(LIBUUID_LIBRARIES "${BREW_PREFIX}/Cellar/util-linux/2.38.1/lib/libuuid.1.dylib")
endif()

include_directories(include ${CMAKE_CURRENT_BINARY_DIR})

add_custom_command(
    OUTPUT ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    COMMAND xxd -i builtin.conf > ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    DEPENDS builtin.conf
)

list(APPEND sources
    ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    src/memfaultctl/crash.c
    src/memfaultctl/memfaultctl.c
    src/memfaultctl/parse_attributes.c
    src/memfaultd.c
    src/queue.c
    src/plugins/attributes/attributes.c
    src/plugins/reboot/reboot.c
    src/plugins/reboot/reboot_last_boot_id.c
    src/plugins/reboot/reboot_process_pstore.c
    src/util/cbor.c
    src/util/config.c
    src/util/device_settings.c
    src/util/disk.c
    src/util/dump_settings.c
    src/util/ipc.c
    src/util/linux_boot_id.c
    src/util/logging.c
    src/util/pid.c
    src/util/plugins.c
    src/util/runtime_config.c
    src/util/rate_limiter.c
    src/util/reboot_reason.c
    src/util/string.c
    src/util/systemd.c
    src/util/version.c
)

if(PLUGIN_SWUPDATE)
    list(APPEND sources src/plugins/swupdate.c)
    add_definitions("-DPLUGIN_SWUPDATE")

    pkg_check_modules(LIBCONFIG REQUIRED libconfig)
    include_directories(include ${LIBCONFIG_INCLUDE_DIRS})

    list(APPEND plugin_libraries ${LIBCONFIG_LIBRARIES})
endif()

if(PLUGIN_COLLECTD)
    list(APPEND sources src/plugins/collectd/collectd.c)
    add_definitions("-DPLUGIN_COLLECTD")
endif()

if(PLUGIN_COREDUMP AND NOT MACOS_BUILD)
    list(APPEND sources
       src/memfault-core-handler/memfault-core-handler.c
       src/memfault-core-handler/core_elf_process_fd.c
       src/memfault-core-handler/core_elf_metadata.c
       src/memfault-core-handler/core_elf_note.c
       src/memfault-core-handler/core_elf_reader.c
       src/memfault-core-handler/core_elf_transformer.c
       src/memfault-core-handler/core_elf_writer.c
       src/memfault-core-handler/coredump_ratelimiter.c
       src/plugins/coredump/coredump.c
    )
    add_definitions("-DPLUGIN_COREDUMP")

    pkg_check_modules(LIBUUID REQUIRED uuid)

    list(APPEND plugin_libraries ${LIBUUID_LIBRARIES} ${ZLIB_LIBRARIES})
endif()

if (PLUGIN_LOGGING)
    add_definitions("-DPLUGIN_LOGGING")
endif()

add_library(memfaultc STATIC ${sources})
add_definitions("-D_DEFAULT_SOURCE=1")

target_compile_options(memfaultc PRIVATE
    -O3
    -g3
    -Wall
    -Wpedantic
    -Wextra
    -Werror
    -Wno-unused-parameter
)
target_include_directories(memfaultc PUBLIC ${JSONC_INCLUDE_DIRS} ${UUID_INCLUDE_DIRS})

install(TARGETS memfaultc LIBRARY DESTINATION lib)
if(TESTS)
    enable_testing()
    add_subdirectory(test)
endif()
