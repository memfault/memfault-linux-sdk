cmake_minimum_required(VERSION 3.16)

project(memfaultd)

option(TESTS "Enable unit tests" ON)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

include(FindPkgConfig)

pkg_check_modules(SDBUS REQUIRED libsystemd)
pkg_check_modules(CURL REQUIRED libcurl)
pkg_check_modules(JSON-C REQUIRED json-c)

include_directories(include ${CMAKE_CURRENT_BINARY_DIR})

add_custom_command(
    OUTPUT ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    COMMAND xxd -i builtin.conf > ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    DEPENDS builtin.conf
)

if(ENABLE_PLUGINS)
    include(plugins/plugins.cmake)
endif()

list(APPEND sources
    ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    src/config.c
    src/device_settings.c
    src/memfaultd.c
    src/memfaultd_utils.c
    src/network.c
    src/queue.c
    src/util/cbor.c
    src/util/disk.c
    src/util/string.c
    src/util/rate_limiter.c
    ${plugin_src}
)

add_executable(memfaultd ${sources})

add_definitions("-D_DEFAULT_SOURCE=1")

target_link_libraries(memfaultd PUBLIC ${CURL_LIBRARIES} ${SDBUS_LIBRARIES} ${JSON-C_LIBRARIES} pthread ${plugin_libraries})
target_compile_options(memfaultd PRIVATE -O3 -Wall -Wpedantic -Wextra -Werror -std=c11)

install(TARGETS memfaultd RUNTIME)
install(FILES ${CMAKE_CURRENT_SOURCE_DIR}/memfaultd.conf TYPE SYSCONF)

if(TESTS)
    enable_testing()
    add_subdirectory(test)
endif()
