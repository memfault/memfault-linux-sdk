if(ENABLE_PLUGINS)
    if(PLUGIN_REBOOT)
        list(APPEND plugin_src reboot.c)
        set(local_plugin_defines "${local_plugin_defines};-DPLUGIN_REBOOT")

        find_package(libuboot)

        list(APPEND plugin_libraries ${libuboot_LIBRARIES})
    endif()

    if(PLUGIN_SWUPDATE)
        list(APPEND plugin_src swupdate.c)
        set(local_plugin_defines "${local_plugin_defines};-DPLUGIN_SWUPDATE")

        include(FindPkgConfig)
        pkg_check_modules(LIBCONFIG REQUIRED libconfig)

        list(APPEND plugin_libraries ${LIBCONFIG_LIBRARIES})
    endif()

    if(PLUGIN_COLLECTD)
        list(APPEND plugin_src collectd.c)
        set(local_plugin_defines "${local_plugin_defines};-DPLUGIN_COLLECTD")
    endif()

    if(plugin_src)
        add_library(plugins ${plugin_src})
        target_link_libraries(plugins ${plugin_libraries})
    endif()

    set(plugin_defines "${local_plugin_defines}" PARENT_SCOPE)
endif()
