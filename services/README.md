# services/

Networking (TCP/IP, TLS 1.3), audio, file systems (FAT32/exFAT), registry, power, graphics (GLES 1.x engine). Layer: System.

| Crate | What | Decision |
|---|---|---|
| `graphics` (`rp-graphics`) | One owner for the 2700G's 2D engine and 3D core | ADR-0030 |
| `display-host` | The display and input driver host | ADR-0016 D5 |
| `audio` (`rp-audio`) | The audio service: one owner of the codec, the I2S link and the speaker amplifier (S1); the mixer, its roles, rings, rewrite-ahead, signal chain and 44.1 kHz resampler (S2) | ADR-0032 |
| `audio-x51v` (`rp-audio-x51v`) | The X51v's audio wiring for the service (codec supply and amplifier through the CPLD, pins, I2C gate, jack), shared by the audio images and later the audio host | ADR-0032 |
| `audio-s1` (`rp-audio-s1`) | The audio service's S1 test image (a guided device run) | ADR-0032 S1 |
| `audio-s2` (`rp-audio-s2`) | The audio service's S2 test image: the mixer, a guided device run | ADR-0032 S2 |

