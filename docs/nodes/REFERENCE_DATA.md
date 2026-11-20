# Reference data the roadmap needs

Status: first pass, 2026-11-19. Nothing was downloaded or fetched for this file (no network).

## Read this first: what this table is built from

The brief asks for every "section 6" entry in the merged `ra-<node>` plans. **No such plan has merged yet.** `docs/nodes/` on main holds only `README.md`, and no branch has a `docs/nodes/<node>.md` plan with a section 6 (checked 2026-11-19). So:

- Part A (the Camry set) is **verified**: it is local, and was read from its `MANIFEST.md` and `SHA256SUMS.txt`.
- Part B (everything else) is **derived from the done checks in `CAPABILITY_ROADMAP.md`**, not from plans. Licences, URLs and sizes there are from memory and are **unverified**; each row says so. Rows must be re-checked against the plans' section 6 once they merge.
- Re-run this task after the plans merge: replace Part B with the plans' real entries and add any data set that Part B misses.

## Part A. Already local: `reference/camry-2013/`

- Location: `reference/camry-2013/` at the repo root. It holds `MANIFEST.md`, `SHA256SUMS.txt`, `ccsa/`, `docs/` and `nhtsa/`.
- **It must stay outside Git.** `/reference/` is listed in `.git/info/exclude`, so it is never committed. Never copy it into a worktree, `shared/` of a repo, a release or anything published.
- Reason: the CCSA models have no stated licence (see below), and the NHTSA files are large binary data.
- Note: the task brief also names a `SOURCES.md` there. **It does not exist.** `MANIFEST.md` holds the source URLs and sizes instead, so this file cites `MANIFEST.md`.
- Integrity: `SHA256SUMS.txt` lists 6 files (CCSA zips and docs); the manifest records all 6 verified PASS on 2026-11-19. The NHTSA hashes are in `MANIFEST.md` only, and also appear in `SHA256SUMS.txt` (added 2026-11-20).

| Data set | Source | Licence | Size on disk | In repo? | Nodes using it |
|---|---|---|---|---|---|
| CCSA 2012 Toyota Camry detailed FE model v5a (LS-DYNA `.key`) | https://www.ccsa.gmu.edu/models/2012-toyota-camry | **None stated. Decision for Lazlo.** | 58,506,561 B zip (248 MB unzipped) | **No** | Crash sim (Camry validation, DEVELOPMENT_PLAN stage 4 step 4); FEA / crash nodes 3.19-3.23 if they use it |
| CCSA 2015 Toyota Camry BM v1 (LS-DYNA `.key`) | https://www.ccsa.gmu.edu/models/2015-toyota-camry | **None stated. Decision for Lazlo.** | 63,550,262 B zip (288 MB unzipped) | **No** | Same as above |
| CCSA tech summary and validation PDFs (2012 Camry) | https://www.ccsa.gmu.edu/wp-content/uploads/2016/06/2012-toyota-camry-tech-summary-v5.pdf and `.../2016/07/2012-toyota-camry-validation-v5.pdf` | Not stated. Decision for Lazlo. | 0.9 MB + 1.1 MB | No | Crash sim validation |
| Toyota 2012 Camry owner's manual PDF | https://assets.sia.toyota.com/publications/en/om-s/OM12K1QRG/pdf/2012%20Camry.pdf | Toyota copyright, no redistribution grant. | 1.9 MB | No | Crash sim (dimensions, reference only) |
| NHTSA report DOT HS 812 635 | https://rosap.ntl.bts.gov/view/dot/41934/dot_41934_DS1.pdf | US Government work, normally public domain; not confirmed here. | 6.8 MB | Probably yes, unconfirmed; keep out until confirmed | Crash sim validation |
| NHTSA test 7520 (2012 Camry LE, frontal NCAP, 56.3 km/h) | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/7520 ; files `https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07520.zip` and `.../iso/vehdb/.../v07520ISO.zip`; also dot.report photos | NHTSA data (US Government work); terms not confirmed here. dot.report photos: unclear, decision for Lazlo. | PDF 9.2 MB, signal zips 3.0 MB + 1.6 MB, 24 photos about 30 MB | Keep out; see decision 3 | Crash sim: frontal validation |
| NHTSA test 8545 (2014 vehicle, frontal NCAP barrier, 56.71 km/h) | same pattern, `v08545*` | Same as 7520 | PDF 25.3 MB, zips 6.5 MB + 4.6 MB | Keep out | Crash sim: frontal validation |
| NHTSA test 7517 (2012 Camry LE, side MDB, 62.3 km/h) | same pattern, `v07517*` | Same as 7520 | PDF 11.5 MB, zips 0.8 MB + 0.5 MB | Keep out | Crash sim: side impact |
| NHTSA test 7523 (2012 Camry LE, side pole, 32.1 km/h) | same pattern, `v07523*` | Same as 7520 | PDF 8.8 MB, zips 0.6 MB + 0.4 MB | Keep out | Crash sim: side pole |
| NHTSA test 9121 (R&D oblique offset frontal, 90.46 km/h) | same pattern, `v09121*` | Same as 7520 | PDF 29.3 MB, zips 9.5 MB + 5.5 MB | Keep out | Crash sim: oblique |
| NHTSA test 9124 (R&D left oblique offset frontal, 89.68 km/h) | same pattern, `v09124*` | Same as 7520 | PDF 29.4 MB, zips 9.6 MB + 5.6 MB | Keep out | Crash sim: oblique |

Total on disk for Part A is roughly 320 MB (CCSA zips 122 MB, NHTSA reports 113 MB, signal zips 48 MB, photos 30 MB, other PDFs 11 MB). Per-file sizes and SHA-256 are in `reference/camry-2013/MANIFEST.md`. Signal data for tests 9121 and 9124: EV5 vehicle 1 is the 2491 kg barrier, not the car. UDS, ABF and TDMS variants exist and were not downloaded.

## Part B. Named by the roadmap's done checks (unverified; replace from the plans' section 6)

Every licence, size and URL in this part is from memory, not checked. Download nothing until Lazlo has seen the decisions.

| Data set | Where to get it (to verify) | Licence (to verify) | Rough size | Nodes (CAPABILITY_ROADMAP.md) | Redistributable in a repo? |
|---|---|---|---|---|---|
| Khronos glTF Sample Assets (models plus reference renders) | github.com/KhronosGroup/glTF-Sample-Assets | **Mixed per model** (CC0, CC-BY, CC-BY-NC and others). Decision for Lazlo: choose models by licence. | hundreds of MB for the full set; a chosen subset is far smaller | 2.5 real-time renderer; 2.4 glTF import/export | Only per model, and not the NC ones; keep out of Git, fetch on demand |
| Ghia, Ghia and Shin (1982) lid-driven cavity tables | The paper (J. Comput. Phys. 48); the tables are small numeric data | Published numbers; the paper is copyrighted. Typing the numbers into a test is usual practice but is **a decision for Lazlo**. | under 10 KB | 3.20 CFD | Probably the numbers only, with citation; unclear |
| Cornell box scene and reference image | Cornell University Program of Computer Graphics site | Unclear (data is widely copied, no clear licence). Decision for Lazlo; can be rebuilt from the published dimensions instead. | under 5 MB | 3.5 path tracer | Rebuild from dimensions rather than copy |
| SVG conformance test subset (W3C SVG test suite) | w3.org SVG test suite pages | W3C test suite licences (W3C Software and Document, or 3-clause BSD, depending on suite). Verify. | tens of MB | 1.3 2D vector renderer | Probably yes, with licence file; verify |
| LAPACK/SciPy reference outputs for numerics | Generated locally with SciPy (BSD-3) or LAPACK (modified BSD) | Permissive for the tool; generated outputs are ours | small | 2.8 numerics core | Yes (we generate them) |
| Dam-break benchmark data | Published experiments (for example Martin and Moyce 1952) or a generated reference | Paper numbers: same question as Ghia. | small | 3.3 fluids | Numbers only, with citation; unclear |
| Published COMSOL thermal-expansion tutorial result | COMSOL tutorial documentation | Vendor copyright. **Decision for Lazlo**; prefer an analytic or open reference problem. | small | 3.21 multiphysics | Unclear; avoid copying |
| Test image for colour management matching | To be chosen (an open one, or generated) | Unknown until chosen | small | 1.11 colour management | Generate our own: yes |
| IIHS crash test reports (named in DEVELOPMENT_PLAN stage 4 step 4 as an alternative to NHTSA) | iihs.org | IIHS copyright, terms not checked. Decision for Lazlo. | PDFs only | Crash sim validation | No, unless licence allows |

## Decisions for Lazlo (every non-free or unclear licence)

1. **CCSA Camry models (2012 v5a, 2015 BM v1) and their PDFs:** no stated licence. The manifest says keep them out of the repo and anything published. Question: is local-only research use enough for the crash sim, and may any derived mesh or reduced model be committed?
2. **Toyota owner's manual:** copyright. Local reference only, never committed.
3. **NHTSA tests and DOT HS 812 635:** US Government material, probably public domain, but the terms were not read here. Decide whether the small signal files may go into the repo (they are 0.4 to 10 MB each) or stay local only. The `dot.report` photos are a third-party site; treat as unclear.
4. **Khronos glTF samples:** per-model licences; choose the subset and drop CC-BY-NC models.
5. **Ghia, Martin and Moyce, COMSOL, Cornell box, IIHS:** copyrighted sources of small numeric data; decide whether quoting the numbers with a citation is acceptable.

## Disk planning

- Part A is about 320 MB, already on disk, and outside every repo.
- Part B downloads are unknown until section 6 arrives; the full Khronos sample set would be the largest item.
- Check free space against `disk_floor_gb` in `broadsword.toml` before any download, and place downloads under `reference/` (outside Git) or `shared/`, never inside a worktree.

## Follow-up needed

- Re-run after the `ra-<node>` plans merge; add every section-6 entry, with the plan file as the source of each row.
- A reviewer spot-check of five entries against merged plans cannot happen until then (done check 1 is **not** met by this first pass; see Part B).
