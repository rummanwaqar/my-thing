SUMMARY = "my-thing runtime"
DESCRIPTION = "BSP runtime + Weston + the my-thing app. The kiosk example's core group minus Chromium and the webapp."
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

inherit packagegroup

PACKAGES = "${PN}"

RDEPENDS:${PN} = " \
    packagegroup-superbird-runtime \
    \
    my-thing \
    \
    mesa \
    weston \
    blank-cursor \
    cursor-suppress \
    superbird-fbpaint \
    \
    htop \
    evtest \
    strace \
"
