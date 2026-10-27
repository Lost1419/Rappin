#!/bin/sh
# Downloads the trusted official SDK and installs the optional Node voice dependency.
set -eu
voice_dir=${1:-.rappin-voice}
mkdir -p "$voice_dir"
curl --fail --location --silent --show-error https://www.yapin.gg/sdk/yapper.js -o "$voice_dir/yapper.mjs"
npm install --cache "$voice_dir/.npm-cache" --prefix "$voice_dir" --save-exact @livekit/rtc-node ffmpeg-static
# npm 12 blocks dependency install scripts until the specific package is approved.
if ! test -x "$voice_dir/node_modules/ffmpeg-static/ffmpeg"; then
    npm --cache "$voice_dir/.npm-cache" --prefix "$voice_dir" install-scripts approve ffmpeg-static
    if ! test -x "$voice_dir/node_modules/ffmpeg-static/ffmpeg"; then
        npm --cache "$voice_dir/.npm-cache" --prefix "$voice_dir" rebuild ffmpeg-static
    fi
fi
"$voice_dir/node_modules/ffmpeg-static/ffmpeg" -version >/dev/null
printf 'Voice runtime ready in %s (includes a local ffmpeg binary).\n' "$voice_dir"
