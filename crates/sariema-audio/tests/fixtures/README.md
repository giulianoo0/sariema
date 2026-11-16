Generated test sounds, no game audio. Both are 2 s of stereo at 44.1 kHz:
silence, then a 440 Hz sine at −12 dBFS from frame 22 050.

```sh
EXPR="if(gte(n\,22050)\,0.25*sin(2*PI*440*t)\,0)"
ffmpeg -f lavfi -i "aevalsrc=${EXPR}|${EXPR}:s=44100:d=2" -c:a libmp3lame -b:a 128k tone.mp3
ffmpeg -f lavfi -i "aevalsrc=${EXPR}|${EXPR}:s=44100:d=2" -c:a vorbis -strict -2 -ac 2 tone.ogg
```
