# JSON-C_FOUND - true if library and headers were found
# JSON-C_INCLUDE_DIRS - include directories
# JSON-C_LIBRARIES - library directories

find_path(libuboot_INCLUDE_DIR libuboot.h)
find_library(libuboot_LIBRARY NAMES libubootenv.so)

set(libuboot_LIBRARIES ${libuboot_LIBRARY})
set(libuboot_INCLUDE_DIRS ${libuboot_INCLUDE_DIR})

include(FindPackageHandleStandardArgs)

find_package_handle_standard_args(libuboot DEFAULT_MSG libuboot_LIBRARY libuboot_INCLUDE_DIR)

mark_as_advanced(libuboot_INCLUDE_DIR libuboot_LIBRARY)
