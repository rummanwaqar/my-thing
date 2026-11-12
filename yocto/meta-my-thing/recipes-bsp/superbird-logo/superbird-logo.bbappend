# u-boot boot splash: 480x800 portrait BMP, artwork rotated 90° clockwise (u-boot doesn't rotate).
# Generated from the landscape splash with:
#   magick boot-splash.png -background '#111111' -alpha remove -rotate 90 -alpha set \
#     -type TrueColorAlpha -define bmp:subtype=ARGB8888 BMP:my-thing-bootup.bmp
FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SUPERBIRD_BOOT_LOGO_NAME = "my-thing-bootup.bmp"
