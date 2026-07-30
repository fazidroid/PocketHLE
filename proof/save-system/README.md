# Save-system verification

The PNGs in this directory are the before/after gameplay captures used to
verify that the supplied Bejeweled CAB still boots after the persistence work.
The run log and API trace are kept outside the repository because they contain
large generated diagnostics.

- `frame_000000.png`: loading screen before the game creates a profile.
- `frame_000001.png` and `frame_000002.png`: profile-entry screen after the
  save directory and CAB registry metadata are available.

The automated tests additionally cover a missing save, a round trip, atomic
replacement, and checksum-detected corruption.
