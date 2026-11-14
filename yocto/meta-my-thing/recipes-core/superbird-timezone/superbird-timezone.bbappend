# my-thing: the device's timezone, from MY_THING_TIMEZONE (conf/distro/my-thing-distro.conf).
# The BSP's tmpfiles line creates /var/lib/timezone/localtime -> UTC only if it's
# missing ("L"). "L+" replaces it on every boot, so the image's setting wins: the
# device has no UI to change the timezone anyway.
do_install:append() {
    conf=${D}${libdir}/tmpfiles.d/superbird-timezone.conf
    sed -i 's|^L .*/var/lib/timezone/localtime .*$|L+ /var/lib/timezone/localtime - - - - /usr/share/zoneinfo/${MY_THING_TIMEZONE}|' $conf
    grep -q "^L+ .*zoneinfo/${MY_THING_TIMEZONE}\$" $conf || bbfatal "superbird-timezone: didn't find the localtime line to change in $conf"
}
do_install[vardeps] += "MY_THING_TIMEZONE"
