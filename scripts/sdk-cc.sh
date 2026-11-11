#!/bin/sh
# Cross C compiler / linker for the Car Thing (aarch64-poky-linux).
# Cargo's `linker` (and cc-rs's CC_<target>) must be a single program, but the
# SDK's $CC is a compiler plus flags (--sysroot, -mcpu, ...). This wraps it.

# The SDK env script refuses to run (and leaves CC unset) if LD_LIBRARY_PATH is
# set, and rustc sets it when invoking the linker. Hide it while sourcing, then
# restore it so the linker runs in the environment rustc intended.
saved_ldlp="${LD_LIBRARY_PATH-}"
unset LD_LIBRARY_PATH
. "${SUPERBIRD_SDK:-$(dirname "$0")/../sdk}/environment-setup-cortexa53-poky-linux"
if [ -z "${CC-}" ]; then
    echo "sdk-cc.sh: SDK environment did not set CC" >&2
    exit 1
fi
[ -n "$saved_ldlp" ] && export LD_LIBRARY_PATH="$saved_ldlp"

exec $CC "$@"    # unquoted on purpose: $CC holds flags
