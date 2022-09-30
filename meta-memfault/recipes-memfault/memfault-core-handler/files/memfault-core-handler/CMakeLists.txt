cmake_minimum_required(VERSION 3.16)

project(memfault-core-handler)

add_executable(memfault-core-handler
    memfault-core-handler.c
)

target_compile_options(memfault-core-handler PRIVATE -O3 -Wall -Wpedantic -Wextra -Werror)

install(PROGRAMS ${CMAKE_BINARY_DIR}/memfault-core-handler TYPE SBIN)
