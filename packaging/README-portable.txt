ByteMap 0.1.0 - portable (no installer)
=======================================

1. Extract this folder anywhere you like.
2. Run bytemap.exe. Windows 11 (x64) with the WebView2 runtime is required;
   Windows 11 already ships it.
3. Keep bytemap-helper.exe in this folder. ByteMap launches it for the fast
   MFT scan, and Windows asks for administrator consent at that moment. If you
   decline, ByteMap falls back to the normal folder walker.
4. Nothing is installed. Snapshots and CSV exports are saved under
   %LOCALAPPDATA%\ByteMap\. To remove ByteMap, delete this folder.
5. This build is unsigned, so Windows SmartScreen may warn on first run.

Licence: MIT OR Apache-2.0. See LICENSE-MIT.txt and LICENSE-APACHE.txt.
