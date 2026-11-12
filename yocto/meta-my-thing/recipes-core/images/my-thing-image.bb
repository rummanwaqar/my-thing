SUMMARY = "my-thing image"
DESCRIPTION = "BSP + Weston kiosk shell + the my-thing app as a systemd service. No Chromium."
LICENSE = "MIT"

inherit core-image
inherit superbird-headroom-check
inherit superbird-image
inherit mainline-flashthing

# splice this image's bandaid.ext4 (the app binary) into the flashthing zip
MAINLINE_FLASHTHING_WITH_BANDAID = "1"
MAINLINE_FLASHTHING_BANDAID_RECIPE = "my-thing-bandaid"

IMAGE_FEATURES += "ssh-server-openssh post-install-logging"

IMAGE_INSTALL = " \
    packagegroup-core-boot \
    packagegroup-my-thing \
    superbird-weston-init-kiosk \
"

BAD_RECOMMENDATIONS += "kernel-modules udev-hwdb wpa-supplicant wireless-regdb wireless-regdb-static weston-init"
BAD_RECOMMENDATIONS += "adwaita-icon-theme-symbolic"

SUPERBIRD_ROOTFS_TYPE = "ext4"
EXTRA_IMAGECMD:ext4 = "-i 16384 -m 0 -O ^has_journal -O ^huge_file"
IMAGE_OVERHEAD_FACTOR = "1.0"
IMAGE_ROOTFS_EXTRA_SPACE = "4096"

IMAGE_FSTYPES = "wic ${SUPERBIRD_ROOTFS_TYPE}"

WKS_FILE = "superbird-mainline.wks.in"

do_image_wic[depends] += " \
    virtual/kernel:do_deploy \
    superbird-uenv:do_deploy \
    superbird-extlinux:do_deploy \
    superbird-logo:do_deploy \
"

EXTRA_IMAGEDEPENDS += "superbird-uboot my-thing-bandaid"
