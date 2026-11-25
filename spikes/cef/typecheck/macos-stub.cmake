# Only for type-checking the macOS target on Linux with stub compilers (see README.md):
# the stub `ar` writes an empty archive, which the host's ranlib rejects. Not used on a Mac.
set(CMAKE_SYSTEM_NAME Darwin)
set(CMAKE_SYSTEM_PROCESSOR arm64)
set(CMAKE_C_COMPILER "$ENV{CC_aarch64_apple_darwin}")
set(CMAKE_CXX_COMPILER "$ENV{CC_aarch64_apple_darwin}")
set(CMAKE_AR "$ENV{AR_aarch64_apple_darwin}" CACHE FILEPATH "stub ar" FORCE)
set(CMAKE_RANLIB /bin/true CACHE FILEPATH "no-op ranlib for the stub archive" FORCE)
