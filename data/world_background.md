# `world_background.txt`: how it was built, and how to touch it again

`world_background.txt` is pre-rasterized ASCII/Unicode landmass art,
embedded verbatim into the binary via `include_str!` in `src/layout.rs`.
Nothing in this repo *generates* it at build time — it's a static asset,
like `data/standard_map.json` — which means the pipeline that produced it,
and the reasoning behind a long list of hand-tuned adjustments, lives only
in this document (and git history) rather than in any Rust source file.
This is written so a future round of adjustments doesn't have to
rediscover all of that from scratch.

It's paired with two other things that were built through the exact same
process and should be read together with it:

- Every country's `"world_cell"` in `data/standard_layout.json` — the
  grid position of its label on this map.
- The region colour tinting in `src/render/worldmap.rs` — this part *is*
  Rust, and documents its own current behaviour in code comments, but
  section 6 below explains the two approaches that were tried and
  discarded first, and why.

## 1. Source data

The landmass shapes come from Natural Earth's 110m land dataset (public
domain / CC0), via the `world-atlas@2` npm package's `land-110m.json`
TopoJSON file. It was fetched once with `curl` into a throwaway script and
decoded by hand (TopoJSON's delta-encoded arcs plus an affine transform)
rather than pulling in a TopoJSON/GeoJSON crate into the project — the
*output* of that decode is a static asset, not runtime code, so there was
no reason to add a permanent dependency for it.

## 2. Rasterization

A small hand-rolled point-in-polygon test (ray casting), supersampled per
output cell to get shading *density* rather than a flat land/sea boolean
— `' '` sea, `'.'` sparse land, `'▒'` medium, `'▓'` dense.

Two bugs worth knowing about if you regenerate this:

- **Antimeridian wraparound.** The combined Europe/Asia/Africa polygon
  crosses ±180° longitude near Siberia. Naive ray casting against it
  misread a chunk of open North Atlantic as land, because a ring that
  crosses the antimeridian corrupts the ray-casting math for query points
  far away. Fixed by unwrapping each ring's longitudes (tracking a
  cumulative ±360° offset as you walk its points) before testing.
- **Arctic distortion.** Left in, Greenland and the Arctic islands
  stretch hugely at high latitude and visually bridge Canada to
  Scandinavia. Excluded by dropping any polygon with a minimum latitude
  above 65°, *plus* Greenland specifically by its own polygon index
  (124 in that dataset) — its own minimum latitude (~60°) didn't trip the
  general rule. Antarctica was dropped outright (out of scope for the TS
  board), and the northern crop was lowered from 78°N to 72°N for the
  same reason.

## 3. The coordinate warp — read this before changing anything

**This is not a real equirectangular projection.** The map is a
deliberate cartogram: real longitude/latitude is bent through a
piecewise-linear warp so that crowded regions (Europe, Central America)
get more room and empty ocean gets less, per an explicit "it doesn't need
to be geographically accurate, take liberties" decision made during
development. Every country's `world_cell` *and* the background raster
were generated together, through the same warp — a `LON_BP` and `LAT_BP`
table of `(real coordinate, output row/col)` breakpoints, with
`piecewise`/`inv_piecewise` helpers to map a real `(lon, lat)` to a grid
`(row, col)` and back. (`LAT_BP` runs in descending real-latitude order,
unlike `LON_BP` — the lookup helper had to detect that and reverse the
table before its bisect search.)

None of that warp code is in this repo; it lived in throwaway scripts.
What *is* preserved is the reasoning behind each hand-tuned adjustment
made to it, because the same class of mistake is easy to repeat:

- **"Africa looks weirdly wide, merging with the Middle East."** Cause: a
  longitude-compression zone (`[-15, 40]`) had been widened to make room
  for Europe. Africa and Europe share almost the same real longitude
  range, so widening one widened both. Fix: dial back that zone's
  expansion factor rather than reverting it outright — a compromise,
  since Europe and Africa aren't longitude-separable in real geography.
- **"South America is a bit narrow."** Cause: an Atlantic-compression
  zone (`[-60, -15]`) meant to shrink empty ocean also compressed
  Brazil/Paraguay/Uruguay, whose real longitudes fall inside it. Fix:
  moved the zone's western boundary from -60 to -45, giving South
  America's own eastern countries a proper expanded zone instead of
  ocean-level compression.
- **"Shrink USA/Canada vertically."** Reduced the two northern latitude
  row budgets (5→4 rows, 10→9 rows) and gave the freed 2 rows to the
  southern zone (21→23); the USA superpower box was also shrunk from 4
  to 3 rows tall.
- **"Europe is too narrow" (a board-topology complaint, not a geographic
  one).** A *local, bounded* stretch was applied instead of redefining a
  global zone: anchored at West Germany's column (88), with an
  asymmetric west stretch factor (2.3) and east stretch factor (1.15),
  active only inside an interior column window (~76.5–104.1) with an
  identity mapping outside it — so Africa and everything else stayed
  untouched. **The important lesson from this round:** the first attempt
  only re-derived the *country labels'* positions through this stretch
  and forgot to re-warp the background raster underneath them, so the
  labels floated over unmodified coastline art — visually indistinguish-
  able from doing nothing. The fix re-applied the same (inverted) stretch
  to the background raster itself, but restricted to Europe's own
  latitude row band (rows 0–12), and was verified by diffing the
  background line-by-line before/after to confirm every row outside that
  band was byte-identical.

**Takeaway for next time:** a purely 1D (longitude-only or
latitude-only) warp can't cleanly separate two regions that share a real
longitude or latitude range — check the affected countries' actual
lon/lat ranges against a zone's boundaries *before* changing it, and
prefer a local, bounded stretch (identity outside a window) over
widening a global zone.

## 4. Placing each country on the grid

- Each country's real approximate `(lon, lat)` is warped through section
  3, then adjusted by a collision-avoidance search: search a ring of
  candidate offsets around the target cell, reserving a footprint (flag +
  code text width, plus a buffer) in a shared "used" set, until an
  unoccupied spot is found.
- A land-check pass then verifies the chosen cell actually renders as
  land (using the same supersampled coverage test as the rasterizer);
  if not, it relocates to the nearest land cell within a small search
  radius. This caught several borderline cases across different rounds —
  Uruguay, Bulgaria, Japan, the Dominican Republic, Thailand/Cambodia,
  Costa Rica/Panama.
- Chips are rendered centred on their target column
  (`start = col - (code.len()+1)/2`), not left-aligned from it —
  left-aligned labels visually drift right of their true point, most
  noticeable wherever countries are dense (Europe).
- Country codes are English mnemonics rather than ISO 2-letter codes
  (`"Fra"` not `"FR"`), hand-checked for confusing or unfortunate
  readings across the full list (`"Chn"` China vs `"Chl"` Chile, `"Jpn"`
  not `"Jap"`, `"Ngr"` Nigeria not `"Nig"`).

## 5. Region colour tinting (`src/render/worldmap.rs`)

This part of the pipeline *is* in Rust and self-documents its current
behaviour, but it went through two failed approaches first, and the
reasoning for discarding each is worth keeping — the underlying tension
(the cartogram warp means "nearest point on the grid" and "actually the
same *Twilight Struggle* region" frequently disagree at region
boundaries) will resurface with any future tweak here.

1. **Per-country Voronoi, no overrides.** Every one of the ~86 countries
   as its own anchor point, tinted by its region. Broke down for the two
   superpowers' home territory, which has no country of its own at all:
   the huge blank landmass around/under the USA and USSR boxes got
   claimed by whichever real country happened to be nearest, however
   implausible (Central America under the USA box; Middle East/Asia
   under the USSR box).
2. **Per-region centroids.** One averaged point per region instead of
   per-country, to smooth out that noise. This helped the superpower-box
   bleed but broke border countries instead: Turkey, Bulgaria and Romania
   are scored as Europe in *Twilight Struggle*, but are geographically
   much closer to the Middle East cluster than to the bulk of Europe —
   so a centroid-only lookup tinted them Middle East, contradicting the
   actual game rule.
3. **What's there now:** back to one anchor point per *country* at its
   own `world_cell` (`tint_zones`), coloured by its *scoring* region —
   this guarantees a country's own position always resolves to itself
   first (zero distance beats any neighbour's), which is exactly what
   fixes Turkey/Bulgaria/Romania. Only the genuinely empty land *between*
   countries is left to a nearest-neighbour guess, a much smaller and
   more forgivable source of error than mislabeling an actual country.
   USA, USSR and Canada are then each carved out as an explicit,
   hardcoded rectangle (`USA_TERRITORY`, `USSR_TERRITORY`,
   `CANADA_TERRITORY`) and force-coloured outright, bypassing the Voronoi
   tessellation entirely:
   - USA/USSR have no country of their own in that area at all, so
     there's no anchor point to give them a fair shot in a
     distance-based contest.
   - Canada does have its own point, but its coastline on this raster is
     far wider than one point can dominate by nearest-neighbour alone —
     its eastern coast sits closer, by grid column, to the Caribbean's
     countries than to Canada's own label.

   Every rectangle's bounds were checked by hand against the full
   `world_cell` list in `data/standard_layout.json` (see each constant's
   doc comment in `worldmap.rs`) to confirm no real country ever falls
   inside one and gets force-coloured to the wrong thing.

4. **A follow-up seam: the empty strip directly south of the USSR box**
   (roughly where real-world Kazakhstan sits — east of Romania/Bulgaria,
   north of Turkey/Iraq) has no country of its own either, and its
   nearest real country is usually Iraq or Turkey, so it tinted Middle
   East — which read as the Middle East abutting the USSR box directly,
   with no Asia in between. Fixed the same way as USA/USSR/Canada: a
   fourth override rectangle, `CENTRAL_ASIA_GAP` (rows 9–10, east of
   column 103), force-coloured Asia instead, bounded to stop before
   Turkey's own row (11) so it doesn't touch a real country. This is the
   general pattern worth remembering: *any* empty gap between two
   regions with no country actually claiming it is a candidate for this
   kind of explicit override, rather than trying to tune the warp or the
   Voronoi to get it "naturally" right.

## 6. If you need to touch this again

- **To regenerate `world_background.txt` from scratch:** re-fetch
  `world-atlas@2`'s `land-110m.json`, redo the TopoJSON decode and the
  antimeridian-safe point-in-polygon rasterization (section 2), then
  re-apply the piecewise lon/lat warp (section 3). None of that pipeline
  is checked into this repo — only its output is — so budget time to
  re-derive the breakpoint tables and stretch factors, tuned by hand
  against the same visual complaints listed above.
- **To move one country's label:** just edit its `"world_cell"` in
  `data/standard_layout.json` directly — it's a plain grid coordinate at
  that point, it doesn't need to go back through the warp. Check it
  lands on a shaded (`'▒'`/`'▓'`) cell in the background art, not a blank
  sea cell (`cargo test` will catch an out-of-bounds cell, but not a
  cell that lands in the sea).
- **To adjust a region's colour tint boundary:** see `tint_zones` /
  `home_territory` / `nearest_zone` in `src/render/worldmap.rs`. Remember
  section 5's lesson — a country's own position is always safe (it
  self-selects), but any large area with *no* country in it (an empty
  ocean gap, a superpower's home territory) needs an explicit override
  rather than trusting the Voronoi tessellation to guess sensibly.
- `data/backup/` holds two earlier full snapshots (pre Africa/South
  America fix, and pre Europe-widening) to compare against or revert to
  if a future change needs one.
