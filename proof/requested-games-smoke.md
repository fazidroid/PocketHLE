# Requested CAB support smoke matrix

The eight uploaded CABs were inspected and run through the same `pockethle run` archive path. The launcher now restores the executable and long asset names from `_setup.xml`, prefers the install shortcut over helper binaries, mounts the recorded install directory, and carries the selected screen geometry into desktop and Android sessions.

| CAB | Selected executable | Result observed |
|---|---|---|
| Bubble Breaker | `BubbleBreaker.exe` | Imports as WVGA; currently stops in the legacy ARM startup path before first frame (`frame_counter=0`). |
| Ferrari | `Ferrari.exe` | Reaches the runtime loop; remaining blocker is dynamic `GetProcAddressA`/legacy CRT initialization (`frame_counter=0`). |
| Prince of Persia HD | `PrinceOfPersiaHD.exe` | Loads correct assets and reaches the runtime loop; no frame yet in the current smoke budget. |
| Diamond Twister | `Diamond Twister.exe` | Reaches the runtime loop (`frame_counter=1`). |
| Splinter Cell Conviction | `Splinter Cell Conviction.exe` | Reaches the runtime loop (`frame_counter=18`). |
| Resident Evil Uprising | `Resident Evil(R) Uprising.exe` | Reaches GDI initialization; then fails on worker-thread stack growth (`WRITE_UNMAPPED` near `0x61ffe7e8`). |
| Crazy Taxi | `Crazy Taxi.exe` | Runs and renders (`frame_counter=36679` in the extended probe). |
| Big Range Hunting 3D | `Hunting3D.exe` | Reaches startup, then fails on a null callback after repeated missing `memchr`/dynamic initialization calls (`frame_counter=0`). |

The checked-in screenshots under `proof/asphalt4-wvga/` remain the visual rendering proof for the existing WVGA path. The matrix deliberately records partial smoke results rather than claiming all eight games are already confirmed playable.
