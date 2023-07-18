cmake_minimum_required(VERSION 3.18)
include(CppUTest.cmake)

set(SRC_DIR ../src)

pkg_check_modules(ZLIB REQUIRED IMPORTED_TARGET zlib)

function(add_memfaultd_cpputest_target NAME)
    add_cpputest_target(${NAME} ${ARGN})

    target_include_directories(${NAME} PRIVATE ${SRC_DIR})
endfunction()

#### TEST TARGET DEFINITIONS ####

add_memfaultd_cpputest_target(test_cbor
    cbor.test.cpp
    ${SRC_DIR}/util/cbor.c
)

if(WITH_COREDUMP)
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
endif()

add_memfaultd_cpputest_target(test_rate_limiter
    rate_limiter.test.cpp
    ${SRC_DIR}/util/rate_limiter.c
)
