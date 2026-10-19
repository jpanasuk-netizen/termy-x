# Third-party notices

## mpv (in-app X video playback on Windows)

Termy's X panel plays video inside the app with mpv (https://mpv.io),
a free media player licensed under the GNU General Public License
(GPLv2+ / LGPLv2.1+ parts; see the license texts shipped with mpv).
Termy bundles or locates an *unmodified* mpv binary (`mpv\mpv.exe`
next to `termy.exe`, or an existing mpv installation) and drives it
only through its public `--wid` child-window embedding and JSON IPC
interfaces. mpv source: https://github.com/mpv-player/mpv
Windows builds used for local installs: shinchiro builds via winget
(`shinchiro.mpv`), https://mpv.io/installation/

No mpv code is linked into or modified by Termy.
