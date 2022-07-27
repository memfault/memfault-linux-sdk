do_install:append() {
    echo "{" > ${D}${sysconfdir}/memfaultd.conf
    echo "  \"base_url\": \"${MEMFAULT_BASE_URL}\"," >> ${D}${sysconfdir}/memfaultd.conf
    echo "  \"project_key\": \"${MEMFAULT_PROJECT_KEY}\"," >> ${D}${sysconfdir}/memfaultd.conf
    echo "  \"software_type\": \"${MEMFAULT_SOFTWARE_TYPE}\"," >> ${D}${sysconfdir}/memfaultd.conf
    echo "  \"software_version\": \"${MEMFAULT_SOFTWARE_VERSION}\"" >> ${D}${sysconfdir}/memfaultd.conf
    echo "}" >> ${D}${sysconfdir}/memfaultd.conf
}
