# my-thing: the device has internet through its USB host (my-thing-uplink), so
# systemd-timesyncd can sync over NTP. The BSP masks timesyncd because it had no
# network path; remove that mask and point it at public NTP servers.
# superbird-clock itself stays: it bumps the clock forward at boot (to the last
# saved time or the image build time) until NTP corrects it.
FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI += "file://my-thing-timesyncd.conf"

do_install:append() {
    rm -f ${D}${sysconfdir}/systemd/system/systemd-timesyncd.service
    # The mask was the only thing in there; an empty, unpackaged dir fails QA.
    rmdir ${D}${sysconfdir}/systemd/system 2>/dev/null || true

    install -d ${D}${sysconfdir}/systemd/timesyncd.conf.d
    install -m 0644 ${S}/my-thing-timesyncd.conf ${D}${sysconfdir}/systemd/timesyncd.conf.d/my-thing.conf
}

FILES:${PN}:remove = "${sysconfdir}/systemd/system/systemd-timesyncd.service"
FILES:${PN} += "${sysconfdir}/systemd/timesyncd.conf.d/my-thing.conf"
