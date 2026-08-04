# Sky Force proof

The supplied Windows Mobile CAB is `SkyForce-92cc6eaa3424.cab`. PocketHLE boots its ARM executable, initializes GAPI, renders the loading screen, and reaches the in-game language-selection menu.

## Reproduction

```bash
target/release/pockethle run SkyForce-92cc6eaa3424.cab \
  --cpu unicorn \
  --max-slices 5000000 \
  --instructions-per-slice 100000 \
  --dump-frames-to /tmp/skyforce-frames \
  --max-frames 400 \
  --message-budget 0
```

The important compatibility fix is the WinCE `coredll.dll!keybd_event` export. Sky Force calls it while leaving the benchmark/loading path; the handler now converts the call to a queued key event instead of treating it as an unimplemented import.

Evidence:

- `skyforce-loading.png` — rendered loading screen.
- `skyforce-language-menu.png` — language-selection menu, proving the game passed the loading/benchmark phase and rendered its interactive menu.
- `skyforce-gameplay-proof.png` — earlier frame progression through loading and benchmarking.
