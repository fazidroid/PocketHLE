# Asphalt 4 WVGA proof

This directory records the final headless Unicorn run of the uploaded Windows Mobile CAB `WVGA_A4-b51f34d1f99d.cab`.

- CAB import selected `Asphalt4.exe`, restored the install paths and persisted `screen: "wvga"`.
- The guest reached its startup/game loop; the final log reports `frame_counter=17` instead of zero.
- The renderer resized the initial `240x320` panel to the presented `480x810` surface and produced eight non-empty screenshots.
- `asphalt4-gameplay-proof.png` is the contact sheet; `frame_000000.png` through `frame_000007.png` are the individual PNG captures.
- `run.log` contains the exact command output and `imported-library.json` records the imported settings.

These are PocketHLE emulator screenshots. No Microsoft Windows Phone emulator or physical Windows Phone device is attached to this Linux build environment, so the evidence does not claim a native-device run.
