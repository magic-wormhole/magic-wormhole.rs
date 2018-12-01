Magic-wormhole.rs depends upon the `sodiumoxide` crate for symmetric
cryptography operations. The current sodiumoxide-0.1.0 crate depends upon
libsodium-1.0.15 or newer. `libsodium` is written in C and produces a shared
library; `sodiumoxide` provides Rust bindings to that library. `sodiumoxide`
does not include the C code and depends upon a pre-installed copy of
`libsodium` (it uses the `pkg-config` tool to find the headers at compile
time, and upon the `LD_LIBRARY_PATH` environment variable to find the shared
library at runtime).

If your OS has this already, and pkg-config knows how to find it,
congratulations! 'cargo build' might just work, and you can ignore the rest.
The OS-specific ways to get a suitable version that I know about are:

* use Ubuntu 18.04 'bionic'
* use Debian 'buster' (which is still in testing)
* use Debian 'stretch' but add backports:
  https://backports.debian.org/Instructions/
* for all Ubuntu/Debian flavors, install `libsodium-dev`

If not, you'll need to do something like this (copied from the sodiumoxide
travis.yml):

* LS=$(pwd)/installed_libsodium
* download the libsodium tarball: https://github.com/jedisct1/libsodium/releases/download/1.0.16/libsodium-1.0.16.tar.gz
* check the sha256: I got eeadc7e1e1bcef09680fb4837d448fbdf57224978f865ac1c16745868fbd0533
* tar xzf libsodium-1.0.16.tar.gz
* cd libsodium-1.0.16 && ./configure --prefix=$LS && make && make install && cd ..
* export PKG_CONFIG_PATH=$LS/lib/pkgconfig:$PKG_CONFIG_PATH
* export LD_LIBRARY_PATH=$LS/lib:$LD_LIBRARY_PATH

At this point, 'cargo build' should work. The change to PKG_CONFIG_PATH is
only necessary while building the `sodiumoxide` crate we depend upon, but
unfortunately the LD_LIBRARY_PATH change is required to run any of the
resulting executables. You must set up an equivalent value in any other
shells where you want to run the magic-wormhole code, or applications which
link against it. For this reason, use an OS-provided package of libsodium if
at all possible.
