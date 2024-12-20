//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include <stdio.h>
#include <string.h>

void function_c(char *str) {
    char *crash = NULL;
    *crash = 'a';
}

void function_b(char *str) {
    printf("Function B: Calling function C\n");
    function_c(str);
}

void function_a(char *str) {
    printf("Function A: Calling function B\n");
    function_b(str);
}

int main() {
    // A simple string to pass to the function
    char *long_string = "a";
    printf("Main: Starting program\n");
    function_a(long_string);
    return 0;
}
