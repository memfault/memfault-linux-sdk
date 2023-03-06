cmake_minimum_required(VERSION 3.16)

pkg_check_modules(CPPUTEST REQUIRED IMPORTED_TARGET cpputest)

add_library(cpputest_runner AllTests.cpp)
target_include_directories(cpputest_runner PUBLIC ${CPPUTEST_INCLUDE_DIRS})


set(SANITIZE_FLAGS
    -fsanitize=address
    -fsanitize=undefined
    -fno-sanitize-recover=all
    )

function(add_cpputest_target NAME)
    add_executable(${NAME} ${ARGN})
    if (NOT MACOS_BUILD)
        # On Linux we want to let the linker select a compatible (32/64) version of the lib
        target_link_libraries(${NAME}
            # cpputest_runner has to come before CPPUTEST_LIBRARIES!!!
            cpputest_runner
            ${CPPUTEST_LIBRARIES}
            )
    else()
        # On Mac we need the complete import which will include the exact path to the file
        target_link_libraries(${NAME}
            # cpputest_runner has to come before CPPUTEST_LIBRARIES!!!
            cpputest_runner
            PkgConfig::CPPUTEST
            )
    endif()
    target_compile_options(${NAME} PRIVATE
        -Wall
        -Wextra
        -Werror
        -Wno-unused-private-field
        -Wno-unused-parameter
        -Wno-missing-field-initializers
        -DCPPUTEST_USE_EXTENSIONS=1
        -DCPPUTEST_USE_MEM_LEAK_DETECTION=0  # conflicts with ASAN
        -DCPPUTEST_SANITIZE_ADDRESS=1
        -DMEMFAULT_UNITTEST
        -O0
        -g
        ${SANITIZE_FLAGS}
        )
    target_include_directories(${NAME} PUBLIC ${CPPUTEST_INCLUDE_DIRS})
    target_link_options(${NAME} PRIVATE -g -O0 ${SANITIZE_FLAGS})
    add_test(NAME ${NAME}_ctest COMMAND ${NAME})
endfunction()
