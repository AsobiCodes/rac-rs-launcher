"""Linker / C compiler wrapper: `zig cc` for x86_64 Linux at glibc 2.28.

Used as the cargo linker (CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER) and as the C compiler for the `ring` crate.
- drops --target=* (cc-rs adds it, zig cannot parse the Rust triple)
- replaces -lgcc_s by -lunwind (zig has no libgcc_s; same trick as cargo-zigbuild)
- drops the static libgcc_eh/libgcc flags zig does not have
"""
import os
import subprocess
import sys

import ziglang

GLIBC = os.environ.get("RACL_GLIBC", "2.28")
zig = os.path.join(os.path.dirname(ziglang.__file__), "zig.exe" if os.name == "nt" else "zig")
args = []
for a in sys.argv[1:]:
    if a.startswith("--target="):
        continue
    if a == "-lgcc_s":
        a = "-lunwind"
    if a in ("-lgcc_eh", "-lgcc"):
        continue
    args.append(a)
sys.exit(subprocess.call([zig, "cc", "-target", "x86_64-linux-gnu." + GLIBC] + args))
