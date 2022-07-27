cmake_minimum_required(VERSION 3.18)
include(CppUTest.cmake)

set(SRC_DIR ../src)

function(add_memfaultd_cpputest_target NAME)
    add_cpputest_target(${NAME} ${ARGN})
    target_link_libraries(${NAME} ${CURL_LIBRARIES} ${JSON-C_LIBRARIES})
    target_include_directories(${NAME} PRIVATE ${SRC_DIR})
endfunction()

#### TEST TARGET DEFINITIONS ####

add_memfaultd_cpputest_target(test_queue
    queue.test.cpp
    ${SRC_DIR}/queue.c
    hex2bin.c
)

add_memfaultd_cpputest_target(test_device_settings
    device_settings.test.cpp
    ${SRC_DIR}/device_settings.c
)
