# Your station everywhere: gauges on Now (R157), charts behind every tile (R158)

The owner, 2026-10-31: "I wanted some of these guages to show up in at least
the now screen if you have the station attached, and also anywhere else that
we can find if it makes sense", and "clicking on the fields in Station does
nothing ... like in the tempest app it brings up a chart ... Most modern apps
dont just have empty fields you can click on". This sheet is for his vote;
nothing here is built.

The mock-ups are the device's own: the Now page and the Station screen drawn
by the shell and the 2D engine's model, their draw commands placed in each
layout and rendered once (every piece is drawn by code the X51v runs; the
charts' lines are painted per pixel, as Hourly's graph is). The charts' data
are made up for the drawing.

## Borrow and beat

| Problem | Tempest's app | Phones' weather apps | Windows Mobile | We take | Ours beats it by |
|---|---|---|---|---|---|
| Station readings on the main screen | its own app only | a "local station" tile at most | none | the station's tiles on Now when one is connected | the tiles the Station screen already paints at start (no new art), the forecast's own cards kept or replaced by the owner's pick |
| A tile that does something | each tile opens a chart: the last 3 h, a point every few minutes, zoom to days, a menu of 12 readings, wind's average/gust/lull with direction arrows (seen on his station page, 2026-10-31) | tap opens a detail page | none | a chart per tile, spans and a reading menu, a tapped point read out | drawn once into one picture when opened (no weight while closed); a tap on a point reads it out large; the reading's high and low with their times under it |

## Gauges on Now: three ways (`mockups/station-on-now-choices.png`)

- **A, the station's tiles in the cards' place:** the forecast's hero and
  summary stay; wind (compass), pressure (gauge), UV, rain and lightning
  take the four cards' place; the page scrolls to them.
- **B, the cards kept, the station's tiles below:** nothing on Now moves;
  the station's tiles follow the four cards (shown scrolled).
- **C, your station first:** the station's own card (live temperature,
  feels like, humidity, dew point) on top, the forecast's summary, then its
  tiles.

Cost: none in RAM (the dials are painted once already); the Now page's
picture grows by the tiles' height on the plane path (A and C ~270 px, B
~280 px).

## A tile's chart: two ways (`mockups/station-chart-choices.png`)

- **A, the chart full screen (as Tempest's):** a row of readings (temp,
  wind, rain, pressure, UV, lightning), spans 3 h / day / week, the chart
  with a "now" line, a tapped point read out, the reading's now/high/low
  under it.
- **B, the tile opens in place:** the tapped tile grows on the Station
  screen into its chart (here wind's average and gust with direction along
  the top), the other tiles below; close puts it back.

Cost: one store picture of the chart (A: 380 x 230 px, ~350 KB while
open; B: 400 x 170, ~270 KB), made when opened and given back when closed;
the history comes through the dock relay from Tempest's observations (a
day at a point every 5 minutes is ~300 readings, a few KB).

## Rule 0 options, asked for after the first set (2026-11-01)

The owner asked whether A/B/C and the two chart layouts used rule 0: only
partly (they borrowed Tempest's and the Station screen's pieces without
beating anything). New options, drawn the same way:

- **N1, smart cards** (`mockups/station-n1-n2.png`): Now keeps its cards;
  with a station connected each card and the big temperature show the
  station's live reading (a green dot), the forecast's words, picture and
  high/low beside them. Beats: phones show a model's grid square, Tempest
  only the station; this shows the best number for each thing, nothing
  twice, no more scrolling.
- **N2, what matters now:** a strip of three readings the image picks by
  what is happening (lightning when strikes in the last 3 h, rain rate
  when raining, gusts over 20 mph; else pressure trend, UV, dew point).
  Shown storming and raining.
- **N3, past and future on one line** (`mockups/station-n3-n4.png`): a
  tap on a reading opens one chart: the last day from the station
  (solid), the next from the forecast (dashed), meeting at now, so it
  shows whether the forecast was right today. Nobody joins them.
- **N4, a sheet you scrub:** the chart slides up over the lower two
  thirds (what was tapped stays in view, swipe down to close); a finger
  dragged along the line, or the D-pad, moves a marker and reads the time
  and value large. Drawn once; only the marker moves.

## Built to try (2026-11-01)

The owner asked to try N1-N4 on the X51v before voting, ready to revert
(R163). Built: N1 and N2 on Now while the station's reading is under an
hour old (`STATION_ON_NOW`); N3 and N4 as one sheet from the Now page's big
temperature and wind, pressure and humidity cards and the Station screen's
temperature, wind and pressure tiles (`chart::CHARTS`): 12 hours or 2 days
each side of now, the station's day from Tempest's observations through
the dock relay (`HISTORY`, its rows cut to the nine fields read), the
forecast's hours dashed, the D-pad's left and right or a finger moving the
marker, up and down changing the reading, Close, a tap above or a pull
down closing it. The line is painted into Hourly's graph slot (446 x 112,
under the 2D engine's 256 KiB picture cache). Pressure's day is lifted to
sea level by the difference now (Tempest's history is at the station's
height). Not built: "the forecast said" and a week (R164).

## Votes

- **S1:** gauges on Now: A, B or C.
- **S2:** a tile's chart: A or B.
- **S3:** where else: Today's hero showing the station's temperature when
  one is connected (yes or no). **Voted yes (the owner, 2026-11-01)**: Today's card and the Now panel are one card (`Shell::weather_card`), the station's live temperature on both, so Reveal's first refresh changes nothing.
