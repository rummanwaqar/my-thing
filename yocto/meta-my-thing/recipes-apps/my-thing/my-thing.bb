SUMMARY = "my-thing: Slint Pomodoro app for the Car Thing"
DESCRIPTION = "Runs fullscreen as a Wayland client of the boot Weston. The binary lives at /usr/lib/my-thing/bin/my-thing.current in the rootfs (the factory copy) and on the bandaid partition, bound to /opt/my-thing, so it can be updated over ssh without a reflash."
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

# The source comes from externalsrc (see yocto/kas/my-thing.yml); only local files here.
SRC_URI = "file://my-thing.service"

inherit cargo pkgconfig systemd

# Linked at build time. Everything else is dlopen'ed (see RDEPENDS).
DEPENDS = "fontconfig"

# Fetch crates with cargo itself instead of bitbake's crate fetcher.
do_compile[network] = "1"
CARGO_DISABLE_BITBAKE_VENDORING = "1"
CARGO_BUILD_FLAGS:remove = "--frozen"
CARGO_BUILD_FLAGS:append = " --no-default-features --features device --locked"

SYSTEMD_SERVICE:${PN} = "my-thing.service"
SYSTEMD_AUTO_ENABLE = "enable"

# winit/glutin dlopen these, so the shlib scan can't see them.
RDEPENDS:${PN} += "opt-overlay wayland libxkbcommon libegl-mesa libgles2-mesa"

APP_FLOOR_DIR = "${nonarch_libdir}/my-thing/bin"
OPT_OVERLAY_TARGET = "/opt/my-thing"

do_install() {
    # bind target for opt-overlay@my-thing; the rootfs is read-only, so it must exist here
    install -d ${D}${OPT_OVERLAY_TARGET}

    install -d ${D}${APP_FLOOR_DIR}
    install -m 0755 ${B}/target/${CARGO_TARGET_SUBDIR}/my-thing ${D}${APP_FLOOR_DIR}/my-thing.current

    install -d ${D}${systemd_system_unitdir}
    install -m 0644 ${UNPACKDIR}/my-thing.service ${D}${systemd_system_unitdir}/my-thing.service
}

FILES:${PN} = " \
    ${OPT_OVERLAY_TARGET} \
    ${APP_FLOOR_DIR}/my-thing.current \
    ${systemd_system_unitdir}/my-thing.service \
"
