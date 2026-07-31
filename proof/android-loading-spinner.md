# Android loading spinner regression proof

- `android-loading-spinner-before.jpg` is the supplied device screenshot: the indeterminate Android `ProgressBar` remains centered over Asphalt 4 gameplay.
- The regression was introduced by the OpenGL renderer migration in commit `2d05672`: the frame polling path submitted frames directly and no longer called `paintFrame`, the method that hides `R.id.progress`.
- The fix restores that path through `paintFrame`. The first decoded framebuffer is submitted to OpenGL and the loading view is hidden on the Android UI thread in the same callback.

A new device screenshot after installing the fixed APK is still required for visual confirmation; this environment has no Android SDK, emulator, or connected device.
