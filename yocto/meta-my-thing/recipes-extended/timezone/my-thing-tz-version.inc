# my-thing: a newer IANA tz database than oe-core's (2026a).
# 2026e has British Columbia's move to permanent UTC-7, so America/Vancouver
# is MST -0700 all year; with 2026a the device shows the old winter PST -0800,
# an hour behind. Shared by tzdata (the zone files) and tzcode-native (zic,
# which compiles them), like oe-core's timezone.inc. The LICENSE file is
# unchanged (LIC_FILES_CHKSUM still matches). Drop this once oe-core is at
# 2026e or later.
PV = "2026e"

SRC_URI[tzcode.sha256sum] = "cc3d27ca2a0d8399504551b920970d80af83bfb9c216e8082a15491921935d54"
SRC_URI[tzdata.sha256sum] = "b26882805f26aac59d5b222978e6580484b834ccdc98be89df2f05a6dc53a652"
