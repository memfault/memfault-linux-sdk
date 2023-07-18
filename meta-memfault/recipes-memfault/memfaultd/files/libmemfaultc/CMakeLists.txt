cmake_minimum_required(VERSION 3.16)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_EXTENSIONS ON)
set(CMAKE_CXX_STANDARD 11)

project(memfaultd)

option(TESTS "Enable unit tests" ON)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

find_package(PkgConfig REQUIRED)

include_directories(include ${CMAKE_CURRENT_BINARY_DIR})

list(APPEND sources
    src/crash.c
    src/util/string.c
)

if(WITH_COREDUMP)
    list(APPEND sources
       src/util/cbor.c
       src/util/rate_limiter.c
       src/memfault-core-handler/core_elf_process_fd.c
       src/memfault-core-handler/core_elf_metadata.c
       src/memfault-core-handler/core_elf_note.c
       src/memfault-core-handler/core_elf_reader.c
       src/memfault-core-handler/core_elf_transformer.c
       src/memfault-core-handler/core_elf_writer.c
       src/memfault-core-handler/coredump_ratelimiter.c
    )
    pkg_check_modules(ZLIB REQUIRED IMPORTED_TARGET zlib)
endif()

if(WITH_SYSTEMD)
    list(APPEND sources src/util/systemd.c)

    pkg_check_modules(SDBUS REQUIRED libsystemd)
endif()

if(WITH_SWUPDATE)
    list(APPEND sources src/swupdate.c)

    pkg_check_modules(LIBCONFIG REQUIRED libconfig)
    include_directories(include ${LIBCONFIG_INCLUDE_DIRS})
endif()

add_library(memfaultc STATIC ${sources})

target_compile_options(memfaultc PRIVATE
    -O3
    -g3
    -Wall
    -Wpedantic
    -Wextra
    -Werror
    -Wno-unused-parameter
)

install(TARGETS memfaultc LIBRARY DESTINATION lib)
if(TESTS)
    enable_testing()
    add_subdirectory(test)
endif()
