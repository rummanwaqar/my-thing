SUMMARY = "Internet for the device through its USB host"
DESCRIPTION = "Adds a default route via the host (the BSP's NCM_HOST_IP) and public DNS to the USB-NCM link. The host needs NAT: scripts/host-share-internet.sh in the my-thing repo."
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

SRC_URI = " \
    file://my-thing-uplink.sh \
    file://my-thing-uplink.service \
"
S = "${UNPACKDIR}"

inherit systemd

SYSTEMD_SERVICE:${PN} = "my-thing-uplink.service"
SYSTEMD_AUTO_ENABLE = "enable"

do_install() {
    install -d ${D}${libexecdir}
    install -m 0755 ${S}/my-thing-uplink.sh ${D}${libexecdir}/my-thing-uplink

    install -d ${D}${systemd_system_unitdir}
    install -m 0644 ${S}/my-thing-uplink.service ${D}${systemd_system_unitdir}/my-thing-uplink.service
}

FILES:${PN} = " \
    ${libexecdir}/my-thing-uplink \
    ${systemd_system_unitdir}/my-thing-uplink.service \
"

# Writes a drop-in for the network file superbird-usb-gadget generates.
RDEPENDS:${PN} = "superbird-usb-gadget"
