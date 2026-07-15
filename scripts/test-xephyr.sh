#!/usr/bin/env bash
#
# Test Rustbox inside a nested Xephyr X server.
#
# Usage:
#   ./scripts/test-xephyr.sh            # uses the wallpaper build (default)
#   ./scripts/test-xephyr.sh nowp       # uses the no-wallpaper build
#   ./scripts/test-xephyr.sh kill       # kills the Xephyr + Rustbox test session
#
# The Xephyr instance is launched with DISPLAY pointing at the real host
# X server (the one already on $DISPLAY) so it can open its own window. The
# Rustbox instance is then launched against the nested display (:5 by default).

set -u

DISPLAY_NUM="${RUSTBOX_TEST_DISPLAY:-5}"
HOST_DISPLAY="${DISPLAY:-:0}"
GEOM="${RUSTBOX_TEST_GEOM:-1920x1080}"

XEPHYR_LOG="/tmp/rustbox-xephyr.log"
WM_LOG="/tmp/rustbox-test.log"

kill_session() {
    pkill -f "Xephyr .*-screen .* :${DISPLAY_NUM}" 2>/dev/null
    pkill -f "target/release/rustbox" 2>/dev/null
    pkill -f "target/debug/rustbox" 2>/dev/null
    sleep 1
    echo "Killed test session on :${DISPLAY_NUM}"
}

if [ "${1:-}" = "kill" ]; then
    kill_session
    exit 0
fi

# Build the requested variant.
if [ "${1:-}" = "nowp" ]; then
    echo "Building no-wallpaper release binary..."
    cargo build --release --no-default-features \
        --features "xrender xinerama xrandr xshape" || exit 1
    BIN="target/release/rustbox"
else
    echo "Building wallpaper (default) release binary..."
    cargo build --release || exit 1
    BIN="target/release/rustbox"
fi

# Start Xephyr nested on the host display.
echo "Starting Xephyr :${DISPLAY_NUM} (${GEOM}) on host ${HOST_DISPLAY}..."
DISPLAY="${HOST_DISPLAY}" Xephyr -ac -br -noreset -screen "${GEOM}" ":${DISPLAY_NUM}" \
    > "${XEPHYR_LOG}" 2>&1 &
XEPHYR_PID=$!

# Wait for the socket to appear.
for _ in $(seq 1 20); do
    if [ -S "/tmp/.X11-unix/X${DISPLAY_NUM}" ]; then
        break
    fi
    sleep 0.5
done

if [ ! -S "/tmp/.X11-unix/X${DISPLAY_NUM}" ]; then
    echo "ERROR: Xephyr failed to start. Log:" >&2
    cat "${XEPHYR_LOG}" >&2
    exit 1
fi

echo "Xephyr up on :${DISPLAY_NUM} (pid ${XEPHYR_PID})"

# Launch Rustbox against the nested display.
echo "Starting Rustbox on :${DISPLAY_NUM}..."
DISPLAY=":${DISPLAY_NUM}" "${BIN}" > "${WM_LOG}" 2>&1 &
WM_PID=$!

echo
echo "Test session running:"
echo "  Xephyr :${DISPLAY_NUM}  pid ${XEPHYR_PID}"
echo "  Rustbox           pid ${WM_PID}"
echo "  WM log: ${WM_LOG}"
echo "  Xephyr log: ${XEPHYR_LOG}"
echo
echo "To stop: ./scripts/test-xephyr.sh kill"
echo "To open a terminal inside: DISPLAY=:${DISPLAY_NUM} kitty &"
