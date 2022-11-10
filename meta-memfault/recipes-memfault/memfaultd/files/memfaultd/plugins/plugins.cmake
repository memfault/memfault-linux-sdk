if(PLUGIN_REBOOT)
    list(APPEND plugin_src
        plugins/reboot/reboot.c
        plugins/reboot/reboot_last_boot_id.c
        plugins/reboot/reboot_process_pstore.c
    )
    add_definitions("-DPLUGIN_REBOOT")

    find_package(libuboot)

    list(APPEND plugin_libraries ${libuboot_LIBRARIES})
endif()

if(PLUGIN_SWUPDATE)
    list(APPEND plugin_src plugins/swupdate.c)
    add_definitions("-DPLUGIN_SWUPDATE")

    include(FindPkgConfig)
    pkg_check_modules(LIBCONFIG REQUIRED libconfig)

    list(APPEND plugin_libraries ${LIBCONFIG_LIBRARIES})
endif()

if(PLUGIN_COLLECTD)
    list(APPEND plugin_src plugins/collectd.c)
    add_definitions("-DPLUGIN_COLLECTD")
endif()

if(PLUGIN_COREDUMP)
    list(APPEND plugin_src
        plugins/coredump/coredump.c
        plugins/coredump/core_elf_metadata.c
        plugins/coredump/core_elf_note.c
        plugins/coredump/core_elf_reader.c
        plugins/coredump/core_elf_transformer.c
        plugins/coredump/core_elf_writer.c
    )
    add_definitions("-DPLUGIN_COREDUMP")

    include(FindPkgConfig)
    pkg_check_modules(LIBUUID REQUIRED uuid)

    list(APPEND plugin_libraries ${LIBUUID_LIBRARIES})
endif()
