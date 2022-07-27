cmake_minimum_required(VERSION 3.18)

project(memfaultd)

option(TESTS "Enable unit tests" ON)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

include(FindPkgConfig)

pkg_check_modules(CURL REQUIRED libcurl)
pkg_check_modules(JSON-C REQUIRED json-c)

include_directories(include ${CMAKE_CURRENT_BINARY_DIR})

add_custom_command(
    OUTPUT ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    COMMAND xxd -i builtin.conf > ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    DEPENDS builtin.conf
)

add_executable(memfaultd ${CMAKE_CURRENT_BINARY_DIR}/builtin_conf.h src/config.c src/device_settings.c src/memfaultd.c src/network.c src/queue.c)

if(ENABLE_PLUGINS)
    add_subdirectory(plugins)
    target_link_libraries(memfaultd PRIVATE plugins)
endif()

add_definitions(${plugin_defines})

target_link_libraries(memfaultd PUBLIC ${CURL_LIBRARIES} ${JSON-C_LIBRARIES})

install(TARGETS memfaultd RUNTIME)
install(FILES ${CMAKE_CURRENT_SOURCE_DIR}/memfaultd.conf TYPE SYSCONF)

if(TESTS)
    enable_testing()
    add_subdirectory(test)
endif()
