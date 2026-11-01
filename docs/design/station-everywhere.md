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

## Votes

- **S1:** gauges on Now: A, B or C.
- **S2:** a tile's chart: A or B.
- **S3:** where else: Today's hero showing the station's temperature when
  one is connected (yes or no).
