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
- Integrity: every hash in the table is copied from `SHA256SUMS.txt` where that file lists the path (30 files: the 2 CCSA zips, the 4 docs, and the report PDF, 2 signal zips and API JSON of each of the 6 tests), and otherwise from `MANIFEST.md` (the 24 photos, `r_7520_1.pdf` and `r_7520_2.pdf`). The "Hash taken from" column names the file used for each row. Nothing was recomputed here. Where both files list a path, the values are equal. `MANIFEST.md` and `SHA256SUMS.txt` themselves have no hash recorded. `SHA256SUMS.txt` holds the first 6 hashes dated 2026-11-19 and the NHTSA lines added 2026-11-20 00:28Z, so its first status line is older than the later additions; `MANIFEST.md` still says it lists 6 files, which is out of date.


Groups, with the nodes that use each (the per-file rows below carry source, size, hash and licence):

- CCSA 2012 Camry v5a and 2015 Camry BM v1 (LS-DYNA models, 248 MB and 288 MB unzipped): crash sim Camry validation (DEVELOPMENT_PLAN stage 4 step 4); FEA and crash nodes 3.19-3.23 if they use it. Not in the repo.
- CCSA tech summary and validation PDFs, Toyota 2012 owner manual, NHTSA report DOT HS 812 635: crash sim validation and dimensions, reference only. Not in the repo.
- NHTSA tests 7520 (2012 Camry LE frontal NCAP, 56.3 km/h) and 8545 (2014 vehicle frontal NCAP, 56.71 km/h): crash sim frontal validation.
- NHTSA tests 7517 (side MDB, 62.3 km/h) and 7523 (side pole, 32.1 km/h): crash sim side impact.
- NHTSA tests 9121 and 9124 (R&D oblique offset frontal, 90.46 and 89.68 km/h): crash sim oblique. Keep all NHTSA files out of the repo (decision 3).

| File (under reference/camry-2013/) | Source | Size (bytes) | SHA-256 | Hash taken from | Licence |
|---|---|---|---|---|---|
| `ccsa/2012-toyota-camry-detailed-v5a.zip` | https://media.ccsa.gmu.edu/model/2012-toyota-camry-detailed-v5a.zip | 58,506,561 | `6a1e252e0538eb192a1769845031a9b143653dbf4253ec0cf772a687b6e2968f` | SHA256SUMS.txt | None stated (decision 1) |
| `ccsa/2015-toyota-camry-bm-v1.zip` | https://media.ccsa.gmu.edu/model/2015-toyota-camry-bm-v1.zip | 63,550,262 | `2e424192a6cfc24e55ad8f2dbb40ea7e7c1e1c8d30e3406d4e6ffb39f4d9ed41` | SHA256SUMS.txt | None stated (decision 1) |
| `docs/2012-toyota-camry-tech-summary-v5.pdf` | https://www.ccsa.gmu.edu/wp-content/uploads/2016/06/2012-toyota-camry-tech-summary-v5.pdf | 909,278 | `796850e06ca623d3fd3c13eea3c77deea41ed6321385de9a4b3fe797a6c264b5` | SHA256SUMS.txt | Not stated (decision 1) |
| `docs/2012-toyota-camry-validation-v5.pdf` | https://www.ccsa.gmu.edu/wp-content/uploads/2016/07/2012-toyota-camry-validation-v5.pdf | 1,084,860 | `123f9fe88d55a9ad532b0e5fe9d349561db0a585b9652446646304ee7d28be38` | SHA256SUMS.txt | Not stated (decision 1) |
| `docs/2012 Camry.pdf` | https://assets.sia.toyota.com/publications/en/om-s/OM12K1QRG/pdf/2012%20Camry.pdf | 1,889,224 | `bcec6b0c70a9d91307e2dd6ff5711d4ff98ccc9de67dafca3a58bf2f78cdccb2` | SHA256SUMS.txt | Toyota copyright (decision 2) |
| `docs/DOT-HS-812-635.pdf` | https://rosap.ntl.bts.gov/view/dot/41934/dot_41934_DS1.pdf (Lazlo's browser download, copied in 2026-11-19 ~23:52Z) | 6,787,952 | `df0eacfcd89f371e7c34841f68073e48b5013fcb2acf36d33d6a3c1cc8be5785` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/r_7520_1.pdf` | https://dot.report/r_7520_1.pdf | 9,178,120 | `c988769856cc394d562af3b263fbf5360c0895a99e35801c7483a62c2a052fc7` | MANIFEST.md | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/r_7520_2.pdf` | https://dot.report/r_7520_2.pdf | 118,016 | `aae0ff09d13b54721d780767c4d9583920517847c5fbc890b2408a1675fe9011` | MANIFEST.md | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/p_7520_1.jpg` | https://dot.report/p_7520_1.jpg | 906,511 | `f4486bc200c2b8766f5a28dbf091cfd21e6af080d8484a3b447dc34353cc7f6c` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_2.jpg` | https://dot.report/p_7520_2.jpg | 1,297,891 | `236b55865b4b78cfe9c75c7d62e736fa06190f8af449b2d4c6730140be1bb0f8` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_3.jpg` | https://dot.report/p_7520_3.jpg | 2,580,767 | `b309efc41424275f8b434e95e7d48e0891774845c729d720ac753a89f4c7af24` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_4.jpg` | https://dot.report/p_7520_4.jpg | 2,589,286 | `866b6bd6b08875a29988f37cb490268833f12db362e218cfa93f800bbfcd8f23` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_5.jpg` | https://dot.report/p_7520_5.jpg | 1,680,746 | `3e907dd485f6904a8b35b58efbcdb9d285e362cf85c8dc07a376cc2c5bde040e` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_6.jpg` | https://dot.report/p_7520_6.jpg | 1,961,617 | `d99021118d93ab5b9e54291088c2a494b147a37e876712e775d18b8ed0250928` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_7.jpg` | https://dot.report/p_7520_7.jpg | 709,580 | `8dd6ee598937c6bf2c8044bb323c7681c9442c7994bf91fef6abae7474192903` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_8.jpg` | https://dot.report/p_7520_8.jpg | 916,676 | `e256a4e5ad8d2648f80bc5f4e51591e65c3bd137f553d2cc081f43f273c341dc` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_9.jpg` | https://dot.report/p_7520_9.jpg | 887,781 | `e6a5a22f233326938de9adedf8f64050515e42086a65b46627b434e81766caae` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_10.jpg` | https://dot.report/p_7520_10.jpg | 842,087 | `ba31e7ce7152ab4e6c28902c813aadbf872d6ddb3dfa7a0a8ada19cb7079d5f3` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_11.jpg` | https://dot.report/p_7520_11.jpg | 773,552 | `b36c8fd4f385bcc1c8b9e28c007a80c229ef762195b11ac85a01942990cc1776` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_12.jpg` | https://dot.report/p_7520_12.jpg | 867,369 | `9c6eeb772251f3f0402db5a31b0e382a7a89d40240d1b615ae4a5a88983b8950` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_13.jpg` | https://dot.report/p_7520_13.jpg | 764,482 | `3ed9156096d8901a834ad5acee26d90dd7932eb270219e1874c05c1c338bab19` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_14.jpg` | https://dot.report/p_7520_14.jpg | 876,350 | `abe4a9a75805eb13570eedf67f5dd0498c4a2f072b99542fab909030e9b3894b` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_15.jpg` | https://dot.report/p_7520_15.jpg | 867,819 | `7dab3ea9268364cf2fc5b793f8851e1b96e207858bb8f06ad9814c374920f8d5` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_16.jpg` | https://dot.report/p_7520_16.jpg | 897,951 | `15868516eb48858a09ce2bd62c708daa1059de6f63fa9660c152c7061aadd8c5` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_17.jpg` | https://dot.report/p_7520_17.jpg | 1,017,355 | `0059c7ee840366510260e40068d1d41d5d7a67eaad0f982cd410ed6360cb692c` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_18.jpg` | https://dot.report/p_7520_18.jpg | 1,123,587 | `ac7804b01507527785b17637195d3ea3809cb96e4fbdcbb93d64d1e7df6f58ce` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_19.jpg` | https://dot.report/p_7520_19.jpg | 1,264,336 | `c2da0c14b00e8d3ffb865efb358cd0e2a92c10fdcb200f9dea7240f3926fedf9` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_20.jpg` | https://dot.report/p_7520_20.jpg | 1,297,617 | `2c0035c12e3addd7387c43b7e0930fa5fe74116ad86e76f1c67823e33ae37638` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_21.jpg` | https://dot.report/p_7520_21.jpg | 901,040 | `a91f8853d9472e340e43192b83514ffed59e3eb3597ef5b187e580f9e304ee2f` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_22.jpg` | https://dot.report/p_7520_22.jpg | 874,130 | `5cbf88cabdae20472b47ce6132642f491a4b60e67a65e926dd19147a41d9b04e` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_23.jpg` | https://dot.report/p_7520_23.jpg | 1,419,114 | `334ebee9cc711cde063a24ea6302c50f4efa04d8d30e987106c2d7a7b2bbac78` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/p_7520_24.jpg` | https://dot.report/p_7520_24.jpg | 1,255,244 | `9a6e893ffb7c3dca97f95369183eaee0e2d121354a3f54da18a93529108454b2` | MANIFEST.md | Unclear, third-party site (decision 3) |
| `nhtsa/7520/v07520R001.pdf` | Downloads/v07520R001.pdf (Lazlo's browser) | 9,178,120 | `c988769856cc394d562af3b263fbf5360c0895a99e35801c7483a62c2a052fc7` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/v07520.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07520.zip | 2,969,167 | `1f78e09e1d833bb10ea68a52f4246ed7202698cbb99a452c6ca5984174be3b7e` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/v07520ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v07500/v07520ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07520ISO.zip | 1,568,467 | `a796bfa245d270965eec2b9c99386a73709adfcd8710f36e1b857576277e219e` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7520/api-vehicle-documents-7520.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/7520 | 722 | `f1d5e6921e66a4b3ff07565292f541d542476b01de86c068dcf0d94f2dcbb038` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/8545/v08545R001.pdf` | Downloads/v08545R001.pdf (Lazlo's browser) | 25,342,222 | `5ff5818b4412ef5f78e9b2edb6644de39a326e3eaa5b248dc9f4ee699fb1d502` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/8545/v08545.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v08500/v08545.zip | 6,472,718 | `490c2537dc840be349838a0512b0dcbf4def566f137735d6e5a27d871626b160` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/8545/v08545ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v08500/v08545ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v08500/v08545ISO.zip | 4,639,374 | `f97eeab0eeb2a36bfc912bfcbb6584eaf9575396276c2a23a7772a8000915428` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/8545/api-vehicle-documents-8545.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/8545 | 722 | `7f3829a2258cea6543fa5f1386090b75ba0197968adfd0ea0e15fa8dfb9214aa` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7517/v07517R001.pdf` | Downloads/v07517R001.pdf (Lazlo's browser) | 11,486,100 | `374061943d4ada37a14204a231e91d9f3638409e53f7d3fe9187029e032f87ca` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7517/v07517.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07517.zip | 822,237 | `3a5752ef4ab4e280a816530e8a0c5800e39e609ac0345c7431a63dc0a7c541a1` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7517/v07517ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v07500/v07517ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07517ISO.zip | 528,660 | `f43b335b575c120cc9f8c469d828078bf6013e274a5a34260396954cf51722ce` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7517/api-vehicle-documents-7517.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/7517 | 722 | `72a8de37ba853445bd41c63a375268bca91409ddd4ed9f80acb57a4cc950d2b4` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7523/v07523R001.pdf` | Downloads/v07523R001.pdf (Lazlo's browser) | 8,835,336 | `91a833058116155631bd052050d00e4d3b90d26973af97c5cad1a5694a3f32f7` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7523/v07523.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07523.zip | 622,938 | `1ef8ae7321b05ea4ed909263ff4fee31508ca33d51e47275edf720b48e6ac299` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7523/v07523ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v07500/v07523ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v07500/v07523ISO.zip | 396,513 | `cbb210a1c8e136f0526dee53856940fedaab7ec76b916d0d09a17018701ae2c7` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/7523/api-vehicle-documents-7523.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/7523 | 722 | `372b2b1b9feb87b87734f6fd9244ad9ef761f887016aee765d1125e09998d81b` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9124/v09124R001.pdf` | Downloads/v09124R001.pdf (Lazlo's browser) | 29,350,105 | `f61bebb15ad01fb0d9cf30f61013072ce8ecf372ca001950da1fa53cda53737f` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9124/v09124.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v09100/v09124.zip | 9,553,988 | `221b06cfcaac43dbc773a61e2b2287369d477af8c35ee7688652340581a3e09d` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9124/v09124ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v09100/v09124ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v09100/v09124ISO.zip | 5,555,398 | `6de6c7049c8cb3bc3106d2bf31c87a5869742cb95ef614c0c02e54414458e111` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9124/api-vehicle-documents-9124.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/9124 | 722 | `197227f25aa405ece32d36e2bea563b9da8ec29f7183dfc4fd62e29340f69bab` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9121/v09121R001.pdf` | Downloads/v09121R001.pdf (Lazlo's browser) | 29,271,408 | `c1d16a08fae9f7d24251232d67b1d8fa094c29be737a2dde79a2a3bfda86a72c` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9121/v09121.zip` | https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v09100/v09121.zip | 9,452,616 | `8755a128c4c4d4c70c52137645afd22c64052eadc36da3e994a59ca71b3eb6cc` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9121/v09121ISO.zip` | https://nrd-static.nhtsa.dot.gov/compress/iso/vehdb/v00000/v09100/v09121ISO.zip https://nrd-static.nhtsa.dot.gov/compress/ev/vehdb/v00000/v09100/v09121ISO.zip | 5,469,427 | `4dc6008f973c8c48f75615bf9e5cf68376d6246048a18513ae3574514cd54936` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `nhtsa/9121/api-vehicle-documents-9121.json` | https://nrd.api.nhtsa.dot.gov/nhtsa/vehicle/api/v1/vehicle-documents/test-no/9121 | 722 | `64e8e8f748f48504d643881e762c22ac616c8089670ed6f66980916cc2351d96` | SHA256SUMS.txt | US Government, unconfirmed (decision 3) |
| `MANIFEST.md` | (local record) | 12,707 | no hash recorded | - | Ours |
| `SHA256SUMS.txt` | (local record) | 3,305 | no hash recorded | - | Ours |

Total on disk for Part A is roughly 320 MB (CCSA zips 122 MB, NHTSA reports 113 MB, signal zips 48 MB, photos 30 MB, other PDFs 11 MB). Per-file sizes and SHA-256 are in the table above (source: `MANIFEST.md` and `SHA256SUMS.txt`). Signal data for tests 9121 and 9124: EV5 vehicle 1 is the 2491 kg barrier, not the car. UDS, ABF and TDMS variants exist and were not downloaded.

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
6. **W3C SVG test suite:** the licence differs by suite and was not checked (Part B row marked Verify). Decide which suite subset to use once its licence is read.
7. **Colour-management test image:** licence unknown until an image is chosen. Decision: generate our own image so no licence is needed, or name an open image and check its licence.

## Disk planning

- Part A is about 320 MB, already on disk, and outside every repo.
- Part B downloads are unknown until section 6 arrives; the full Khronos sample set would be the largest item.
- Check free space against `disk_floor_gb` in `broadsword.toml` before any download, and place downloads under `reference/` (outside Git) or `shared/`, never inside a worktree.

## Follow-up needed

- Re-run after the `ra-<node>` plans merge; add every section-6 entry, with the plan file as the source of each row.
- A reviewer spot-check of five entries against merged plans cannot happen until then (done check 1 is **not** met by this first pass; see Part B).
