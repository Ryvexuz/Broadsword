# Reference data the roadmap needs

**Pass 2 of 2 (task t-20261120-015238, 2026-11-20): Part B now comes from the section 6 of the merged node plans, and the five-entry spot-check is done (see "Spot-check (pass 2)" under Part B).** Part A (the files on disk) is unchanged from pass 1.

Status: nothing was downloaded or fetched for this file (no network), in either pass.

## Read this first: what this table is built from

- Part A (the Camry set) is **verified** as of pass 1: it is local, and was read from its `MANIFEST.md` and `SHA256SUMS.txt`.
- Part B is built from the "## 6. Reference data" section of each merged plan in `docs/nodes/`: `1.1`, `1.2`, `1.3`, `1.4`, `1.5`, `1.6`, `1.7`, `1.8`, `2.8` and `sandbox`. `0.1.md` has no section 6. Every row names its plan file and section, and its licence is the plan's own wording. **Those plans say their URLs and licences are from memory; no row has been checked against the live source**, so a licence in Part B is "as the plan states it", not confirmed.
- Part B rows for nodes without a merged plan keep their pass-1 text (from the roadmap's done checks, unverified) and are marked "no plan yet".

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
| `nhtsa/7520/r_7520_1.pdf` | https://dot.report/r_7520_1.pdf | 9,178,120 | `c988769856cc394d562af3b263fbf5360c0895a99e35801c7483a62c2a052fc7` | MANIFEST.md | US Government, unconfirmed (decision 3); same file as `nhtsa/7520/v07520R001.pdf`, byte-identical; kept under both NHTSA names |
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
| `nhtsa/7520/v07520R001.pdf` | Downloads/v07520R001.pdf (Lazlo's browser) | 9,178,120 | `c988769856cc394d562af3b263fbf5360c0895a99e35801c7483a62c2a052fc7` | SHA256SUMS.txt | US Government, unconfirmed (decision 3); same file as `nhtsa/7520/r_7520_1.pdf`, byte-identical; kept under both NHTSA names |
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

## Part B. Reference data named by the merged node plans (section 6) and by the roadmap

Built in pass 2 (2026-11-20) from the "## 6. Reference data" section of each merged plan in `docs/nodes/`, read in this worktree. Nothing was downloaded or fetched. Every licence is copied as the plan states it; **a plan's own words "from memory", "to confirm" and "unverified" carry over to its rows**, so no URL, licence or number in this part has been checked against the live source. Each row names its plan file and section. Size is "not stated" where the plan gives none.

Section 6 per plan: `0.1.md` has **no section 6** (it has no numbered sections; it lists no data set, and its only external pin is the `actions/checkout` SHA, which the plan says to verify at push time). Every other merged plan has one: `1.1`, `1.2`, `1.3`, `1.4`, `1.5`, `1.6`, `1.7`, `1.8`, `2.8`, `sandbox`.

Columns: Data set or source, Used for, Source as the plan gives it, Licence as the plan states it, Size, Plan and section, Status. Status is "plan row" for every row copied from a plan.

### Node 0.1 (docs/nodes/0.1.md): no section 6

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| none | The plan has no section 6 and names no data set. | - | - | - | `0.1.md` (no section 6) | no section 6 |

### Node 1.1 (docs/nodes/1.1.md section 6)

The plan: "Every value below is from memory of Microsoft's documentation and must be confirmed against the page by the brief that uses it (W2 for decode values, W6 for the pointer API, W8 for injection, W10 for XInput)."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| `POINTER_PEN_INFO`: pressure 0-1024, tilt X and Y -90 to 90, rotation 0-359 | pen decode values (brief W2) | https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info | "Microsoft Learn text. The MicrosoftDocs repositories publish docs under CC BY 4.0; confirm before quoting any text. Only numeric facts are used." | not stated | `1.1.md` section 6 | plan row |
| `POINTER_INFO`: `ptPerformanceCount` (hardware timestamp, QueryPerformanceCounter clock) and `dwTime` | pointer API (brief W6) | https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_info | "same" (as the row above) | not stated | `1.1.md` section 6 | plan row |
| `GetPointerPenInfo` and `GetPointerPenInfoHistory` (several samples can arrive in one message) | pointer API (brief W6) | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfo | "same" | not stated | `1.1.md` section 6 | plan row |
| `CreateSyntheticPointerDevice` and `InjectSyntheticPointerInput` | injection (brief W8) | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-injectsyntheticpointerinput | "same" | not stated | `1.1.md` section 6 | plan row |
| `XINPUT_GAMEPAD`: stick range -32768..32767, triggers 0-255, dead zones 7849, 8689 and 30 | XInput (brief W10) | https://learn.microsoft.com/en-us/windows/win32/api/xinput/ns-xinput-xinput_gamepad | "same" | not stated | `1.1.md` section 6 | plan row |
| `QueryPerformanceCounter` and `QueryPerformanceFrequency` | timestamps | https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps | "same" | not stated | `1.1.md` section 6 | plan row |
| `WM_MOUSEWHEEL` (`WHEEL_DELTA` = 120) | wheel decode | https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-mousewheel | "same" | not stated | `1.1.md` section 6 | plan row |
| Virtual-key codes | key decode | https://learn.microsoft.com/en-us/windows/win32/inputdev/virtual-key-codes | "same" | not stated | `1.1.md` section 6 | plan row |
| Windows SDK headers (`winuser.h`, `xinput.h`) | offline cross-check of every constant above | "installed with the Windows SDK on the PC, if present" | "Windows SDK licence; read only, never copied into the repo" | not stated | `1.1.md` section 6 | plan row |
| Sample sets, published latency results, tablet traces | none | "None." Thresholds are budgets and loose human-check bounds. | not applicable | - | `1.1.md` section 6 | plan row: none used |

### Node 1.2 (docs/nodes/1.2.md section 6)

The plan: "List only; nothing is downloaded."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| WGSL specification, including "Floating Point Accuracy" | the source for the float threshold | https://www.w3.org/TR/WGSL/ | "W3C Software and Document licence (as published by the W3C; to confirm when citing)" | not stated | `1.2.md` section 6 | plan row |
| wgpu documentation and examples (pinned version) | API reference; "Used to read, not as test data." | https://docs.rs/wgpu and https://github.com/gfx-rs/wgpu | "MIT OR Apache-2.0" | not stated | `1.2.md` section 6 | plan row |
| external image, mesh or benchmark set | none: inputs are generated in code, expected values come from std-only CPU references in this repo | - | "no third-party data to license" | - | `1.2.md` section 6 | plan row: none used |
| Khronos glTF sample models (node 2.5) and SVG test suite (node 1.3) | "Later nodes may add references ... They are not part of 1.2." | - | - | - | `1.2.md` section 6 | not part of 1.2; see the glTF row under "no plan yet" and the 1.3 rows |

### Node 1.3 (docs/nodes/1.3.md section 6)

The plan: "Listing only: nothing is downloaded or copied in this task. The URLs are from memory and must be confirmed (and the licences read from the source) by whoever picks them." This replaces the pass-1 row "SVG conformance test subset (W3C SVG test suite)".

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| SVG 1.1 (Second Edition) | the specification the subset and the gradient and stroke formulas come from | https://www.w3.org/TR/SVG11/ | "W3C Document licence (to confirm when citing). Only formulas and element names are used; no text is copied." | not stated | `1.3.md` section 6 | plan row |
| W3C SVG 1.1 test suite (cases and reference PNGs) | a candidate for "the SVG conformance suite" | https://www.w3.org/Graphics/SVG/Test/ | "W3C test-suite licence; whether the files may be committed to a repo is **to confirm**." | not stated | `1.3.md` section 6 | plan row; commit question open |
| resvg test suite (SVG files and reference PNGs) | "a second candidate" for renderer tests | https://github.com/linebender/resvg ("location from memory; the project has moved between owners") | "To confirm. The repo is Rust, and its licence for the tests is not stated here from memory." | not stated | `1.3.md` section 6 | plan row; licence unknown |
| PNG specification | the decoder (chunks, filters, zlib and deflate) | https://www.w3.org/TR/png/ and RFC 1950 and RFC 1951 | "W3C and IETF text; formulas and format facts only." | not stated | `1.3.md` section 6 | plan row |
| Bezier flattening and stroking background (error bound, joins) | V2 and V4 | "Any one textbook or paper chosen by V2 and V4 and cited in the file" | "To confirm per source; no text is copied." | not stated | `1.3.md` section 6 | plan row; source not chosen |
| `projects/render/src` (rasterizer and `tests/golden.rs`) | existing code to cross-check by hand: the golden pattern | local path | "Own code." | not stated | `1.3.md` section 6 | plan row |
| published tolerance | none: tolerances in section 1 are starting values (c2 and c12); V10 measures and records it | - | not applicable | - | `1.3.md` section 6 | plan row: none used |

### Node 1.4 (docs/nodes/1.4.md section 6)

The plan: "List only; nothing is downloaded. Licences are from memory and are confirmed when the file is first used."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| Unicode `BidiCharacterTest.txt` and `BidiTest.txt` | conformance data for UAX 9 | https://www.unicode.org/Public/UCD/latest/ucd/BidiCharacterTest.txt and https://www.unicode.org/Public/UCD/latest/ucd/BidiTest.txt | "Unicode terms of use (data files; copyright and permission notice kept in the fixture header)" | not stated | `1.4.md` section 6 | plan row |
| Unicode `GraphemeBreakTest.txt` (UAX 29) and `LineBreakTest.txt` (UAX 14) | cluster and wrap tests in X7 | https://www.unicode.org/Public/UCD/latest/ucd/auxiliary/GraphemeBreakTest.txt and https://www.unicode.org/Public/UCD/latest/ucd/auxiliary/LineBreakTest.txt | "Same licence." | not stated | `1.4.md` section 6 | plan row |
| HarfBuzz shaping tests | inputs and expected glyph output for fonts and features; source of expected ids for check 1.4.3, "only for fonts that are also licensed to vendor" | https://github.com/harfbuzz/harfbuzz/tree/main/test/shaping | "the HarfBuzz licence (an MIT-style licence)" | not stated | `1.4.md` section 6 | plan row |
| rustybuzz tests | same use as the HarfBuzz tests | https://github.com/harfbuzz/rustybuzz | "MIT" | not stated | `1.4.md` section 6 | plan row |
| Noto fonts (Latin, Arabic, Hebrew, Devanagari) | fonts for shaping checks | https://github.com/notofonts and https://fonts.google.com/noto | "SIL Open Font License 1.1 (https://openfontlicense.org). Redistribution in the repo needs the licence text kept with the files." | not stated | `1.4.md` section 6 | plan row |
| OpenType specification (GSUB, GPOS, cmap, hmtx, glyf, CFF) | "Read, not copied." | https://learn.microsoft.com/en-us/typography/opentype/spec/ | not stated in the plan | not stated | `1.4.md` section 6 | plan row; licence not stated |
| Unicode Standard Annexes 9, 14 and 29 | "Read, not copied." | https://www.unicode.org/reports/tr9/ , https://www.unicode.org/reports/tr14/ , https://www.unicode.org/reports/tr29/ | not stated in the plan | not stated | `1.4.md` section 6 | plan row; licence not stated |
| Frame budget 60 fps = 16.67 ms | plain arithmetic | "no source needed" | not applicable | - | `1.4.md` section 6 | plan row |
| external image set | none: the visual reference for 1.4.11 is a browser's rendering of the same text on Lazlo's PC, compared by eye and not stored; the one-million-line document is generated | - | not applicable | - | `1.4.md` section 6 | plan row: none used |

### Node 1.5 (docs/nodes/1.5.md section 6)

The plan: "Listing only: nothing is downloaded or copied into the repo. Facts below are from memory and are confirmed against the page by the brief that uses them."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| 60 Hz frame time (16.67 ms) and the "two frames is a visible hitch" rule | frame budget | "definition (1 s / 60); no external benchmark is compared against" | not applicable | - | `1.5.md` section 6 | plan row |
| The 10,000-edit undo bar | undo check | `CAPABILITY_ROADMAP.md` node 0.6 Done-when (this repo) | not applicable | - | `1.5.md` section 6 | plan row |
| `StretchDIBits` | presenting a CPU framebuffer | https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-stretchdibits | "Microsoft Learn text; numeric and API facts only, no text copied" | not stated | `1.5.md` section 6 | plan row |
| `QueryPerformanceCounter` | frame timing | https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps | "same" | not stated | `1.5.md` section 6 | plan row |
| `timeBeginPeriod` | frame pacing, "if S10 uses it" | https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod | "same" | not stated | `1.5.md` section 6 | plan row |
| `LoadLibraryW` and `GetProcAddress` | plugin host | https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryw | "Microsoft Learn text; API facts only" | not stated | `1.5.md` section 6 | plan row |
| Rust: a panic that reaches an `extern "C"` function boundary aborts the process (stable since Rust 1.81, from memory) | the fact the host-process containment of `bad_panic` relies on | https://doc.rust-lang.org/nomicon/unwinding.html and the Rust 1.81 release notes | "MIT OR Apache-2.0 (Rust documentation); fact only" | not stated | `1.5.md` section 6 | plan row |
| `catch_unwind` | in-process containment of Rust groups | https://doc.rust-lang.org/std/panic/fn.catch_unwind.html | "MIT OR Apache-2.0; fact only" | not stated | `1.5.md` section 6 | plan row |
| PGM (P5) image format | the atlas | https://netpbm.sourceforge.net/doc/pgm.html | "Netpbm documentation; the format is a fact, nothing copied" | not stated | `1.5.md` section 6 | plan row |
| Dear ImGui docking branch; Blender workspaces | design references for docking and per-mode layouts, read only | https://github.com/ocornut/imgui and https://docs.blender.org/manual/en/latest/interface/window_system/workspaces.html | "Dear ImGui is MIT; Blender is GPL (its code is never read for copying, only its manual for ideas)" | not stated | `1.5.md` section 6 | plan row |
| Sample sets, golden images, published UI benchmarks | none: the checks compare the shell with its own saved bytes and with budgets | "None." | not applicable | - | `1.5.md` section 6 | plan row: none used |

### Node 1.6 (docs/nodes/1.6.md section 6)

The plan: "List only; nothing is downloaded. No test data from outside is used, with one exception: the published FNV-1a test vectors below (a few short constants, named with their URL)."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| Rust Reference: `extern` functions and the C ABI, and `repr(C)` | read, for the ABI rules and for the rule that a panic must not unwind across `extern "C"` | https://doc.rust-lang.org/reference/items/external-blocks.html and https://doc.rust-lang.org/reference/type-layout.html | "MIT OR Apache-2.0 (the Rust Reference's own; to confirm when citing)" | not stated | `1.6.md` section 6 | plan row |
| Python `ctypes` documentation | read, for check 1.6.9b | https://docs.python.org/3/library/ctypes.html | "PSF (documentation)" | not stated | `1.6.md` section 6 | plan row |
| FNV-1a hash definition (64 bit) and its test vectors | the descriptor hash constants, as in node 1.2; "test vectors are taken from that page and written into `tests/descriptor.rs` with the URL in a comment" | http://www.isthe.com/chongo/tech/comp/fnv/ | "Public domain (stated on the page; to confirm)" | "a few short constants" | `1.6.md` section 6 | plan row |
| Prior art, "read for ideas only, not code": Unreal `UPROPERTY` documentation, Unity serialization rules, crates `bevy_reflect` and `serde` | design ideas; "None of this is a dependency." | https://dev.epicgames.com/documentation/en-us/unreal-engine/ , https://docs.unity3d.com/Manual/script-Serialization.html , https://docs.rs/bevy_reflect , https://serde.rs | "Epic's and Unity's pages are proprietary documentation (no text is copied); `bevy_reflect` and `serde` are MIT OR Apache-2.0 (from memory; confirm before quoting)" | not stated | `1.6.md` section 6 | plan row |
| every other input | generated in code or written by hand; expected values hand-worked | - | "no other third-party data to licence" | - | `1.6.md` section 6 | plan row: none used |

### Node 1.7 (docs/nodes/1.7.md section 6)

The plan: "Listing only: nothing is downloaded or copied into the repo. This node has **no published numeric benchmark** to compare against ... These URLs are written from memory and are unverified."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| "Build Systems a la Carte", Mokhov, Mitchell and Peyton Jones, 2018 | dirty marking, lazy demand and minimal rebuild as a general idea | https://www.microsoft.com/en-us/research/publication/build-systems-a-la-carte/ | "Paper copyright its authors; read only, no text copied" | not stated | `1.7.md` section 6 | plan row |
| Topological sorting (Kahn's algorithm) | the evaluator and the cycle check | https://en.wikipedia.org/wiki/Topological_sorting | "CC BY-SA 4.0 (facts only, no text copied)" | not stated | `1.7.md` section 6 | plan row |
| Blender Manual, node editors | prior art for typed sockets, node groups and a shared node editor; "used to decide that groups expose inner sockets" | https://docs.blender.org/manual/en/latest/interface/controls/nodes/index.html | "CC BY-SA 4.0 (read only ...)" | not stated | `1.7.md` section 6 | plan row |
| `std::panic::catch_unwind` | what it does and does not catch | https://doc.rust-lang.org/std/panic/fn.catch_unwind.html | "MIT or Apache-2.0 (standard library docs)" | not stated | `1.7.md` section 6 | plan row |
| Linear congruential generator constants (multiplier 6364136223846793005, increment 1442695040888963407) | seeded generator | https://en.wikipedia.org/wiki/Linear_congruential_generator | "CC BY-SA 4.0 (two constants, from memory: confirm against the page before use)" | two constants | `1.7.md` section 6 | plan row |
| The Rustonomicon, FFI chapter | C ABI conventions: `extern "C"`, `#[repr(C)]`, opaque pointers, UTF-8 pointer plus length | https://doc.rust-lang.org/nomicon/ffi.html | "MIT or Apache-2.0" | not stated | `1.7.md` section 6 | plan row |
| Sample sets, published performance results, reference graphs | none: the 50 ms budget is frame-time reasoning; reference graph shapes A and B are defined in section 5 | "None." | not applicable | - | `1.7.md` section 6 | plan row: none used |

### Node 1.8 (docs/nodes/1.8.md section 6)

The plan: "Listing only: nothing is downloaded or copied into the repo. Every value is from memory and must be confirmed against the page by the brief that uses it."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| CSS easing: cubic Bezier `cubic-bezier(x1, y1, x2, y2)` and its solve-for-x rule | easing curve | https://www.w3.org/TR/css-easing-1/ | "W3C document licence; only the formula is used" | not stated | `1.8.md` section 6 | plan row |
| glTF 2.0 animation samplers `STEP`, `LINEAR`, `CUBICSPLINE` and the cubic Hermite formula (Appendix C) | interpolation | https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html | "Khronos specification, Creative Commons Attribution; only the formula is used" | not stated | `1.8.md` section 6 | plan row |
| OpenTimelineIO data model (Timeline, Track, Clip, `RationalTime`, `TimeRange`) | "the model to compare names and edge cases against" | https://github.com/AcademySoftwareFoundation/OpenTimelineIO | "Apache-2.0" | not stated | `1.8.md` section 6 | plan row |
| Blender manual, animation (keyframes, F-curves, interpolation, Graph Editor) | ideas only | https://docs.blender.org/manual/en/latest/animation/index.html | "Creative Commons BY-SA 4.0 (documentation text; only ideas are used)" | not stated | `1.8.md` section 6 | plan row |
| Premiere Pro time base of 254,016,000,000 ticks per second | a number; "t1 checks the property independently" | "Adobe Premiere Pro SDK documentation (page to be found by the brief)" | "Adobe documentation; the figure is a number" | one figure | `1.8.md` section 6 | plan row; no URL |
| NTSC frame rates 24000/1001, 30000/1001, 60000/1001 | rational frame rates | https://en.wikipedia.org/wiki/NTSC | "Creative Commons BY-SA; facts only" | not stated | `1.8.md` section 6 | plan row |
| Audio sample rates 44,100 and 48,000 Hz (AES5) | two numbers | "AES5-2018, paywalled" | "not needed beyond the two numbers; list only" | two numbers | `1.8.md` section 6 | plan row |
| Python `ctypes` | C ABI check | https://docs.python.org/3/library/ctypes.html | "PSF documentation licence" | not stated | `1.8.md` section 6 | plan row |
| Sample sets, published scrub or edit benchmarks | none: thresholds are budgets and chosen stress figures | "None." | not applicable | - | `1.8.md` section 6 | plan row: none used |

### Node 2.8 (docs/nodes/2.8.md section 6)

The plan: "Listing only: nothing is downloaded or copied into the repo. The fixtures are generated locally (D3) ... **Every URL below was written from memory with no network access and is unverified** ... No sizes are given for documents; generated data sizes are in section 4." This replaces the pass-1 row "LAPACK/SciPy reference outputs for numerics", as the plan itself asks. Source and licence cells below carry the plan's "(to verify)" marking.

| Data set | Used for | Source (to verify) | Licence as stated (to verify) | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| SciPy (`scipy.linalg`, `scipy.sparse`, `scipy.fft`, `scipy.integrate.solve_ivp`, `scipy.optimize`) | "The reference outputs of every fixture (N3)." | https://scipy.org and https://docs.scipy.org/doc/scipy/ | "BSD-3-Clause" | not stated | `2.8.md` section 6 | plan row |
| NumPy (`numpy.linalg`, `numpy.random.default_rng`) | determinant, inverse, condition numbers, seeded inputs (N3) | https://numpy.org | "BSD-3-Clause" | not stated | `2.8.md` section 6 | plan row |
| Reference LAPACK, including its test programs and input files (test-ratio threshold, 30 from memory) | `THRESH` and the forms of the test ratios (N2, N5); read only | https://github.com/Reference-LAPACK/lapack (`TESTING/`) | "Modified BSD" | not stated | `2.8.md` section 6 | plan row |
| LAPACK Users' Guide, 3rd edition, chapter "Accuracy and Stability" | forward error bounded by condition number times backward error (section 1); eigenvalue and singular value bounds (n5); read only | https://www.netlib.org/lapack/lug/ | "Copyright SIAM and the authors; free to read online" | not stated | `2.8.md` section 6 | plan row |
| Higham, *Accuracy and Stability of Numerical Algorithms*, 2nd edition, SIAM 2002 | LU and Cholesky backward error; the `eps * log2(n)` growth of FFT error (n7) | "The book (no URL)" | "Copyright; read only" | not stated | `2.8.md` section 6 | plan row; no URL |
| Demmel and Veselic, "Jacobi's method is more accurate than QR", SIAM J. Matrix Anal. Appl. 13(4), 1992 | choice of Jacobi rotations for `eigh` and one-sided Jacobi for SVD (N5) | "The paper (no URL)" | "Copyright; read only" | not stated | `2.8.md` section 6 | plan row; no URL |
| Bluestein's algorithm (chirp-z) | prime and mixed FFT lengths (N7) | https://en.wikipedia.org/wiki/Chirp_Z-transform | "CC BY-SA 4.0 (facts only, no text copied)" | not stated | `2.8.md` section 6 | plan row |
| Dormand and Prince, "A family of embedded Runge-Kutta formulae", J. Comput. Appl. Math. 6(1), 1980 | the DOPRI5 tableau (N8), cross-checked against SciPy's `RK45` source (`scipy/integrate/_ivp/rk.py`, BSD-3) | "The paper (no URL)" | "Copyright; the coefficients are facts" | not stated | `2.8.md` section 6 | plan row; no URL |
| Hairer and Wanner, *Solving Ordinary Differential Equations II*, Springer | the stiff test problem of n10 (Robertson; its equations are facts) | "The book (no URL)" | "Copyright; read only" | not stated | `2.8.md` section 6 | plan row; no URL |
| Hairer, Lubich and Wanner, *Geometric Numerical Integration*, Springer | bounded energy of symplectic methods (n9) | "The book (no URL)" | "Copyright; read only" | not stated | `2.8.md` section 6 | plan row; no URL |
| Rosenbrock function and its derivatives as SciPy ships them | n11, n12 | https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.rosen.html | "BSD-3 (docs)" | not stated | `2.8.md` section 6 | plan row |
| Moré, Garbow and Hillstrom, "Testing Unconstrained Optimization Software", ACM TOMS 7(1), 1981 | optimizer test problems and start points (n12) | "The paper (no URL)" | "Copyright; the problem definitions are facts" | not stated | `2.8.md` section 6 | plan row; no URL |
| MINPACK (Levenberg-Marquardt, behind `least_squares(method='lm')`) | n12's LM comparison; "Read only; not linked." | https://www.netlib.org/minpack/ | "MINPACK licence (BSD-like, to verify)" | not stated | `2.8.md` section 6 | plan row; licence to verify |
| Griewank and Walther, *Evaluating Derivatives*, 2nd edition, SIAM 2008 | forward (dual numbers) and reverse (tape) modes (N9) | "The book (no URL)" | "Copyright; read only" | not stated | `2.8.md` section 6 | plan row; no URL |
| foundation's generator and its published test vector | the copied `Xoshiro256StarStar` (N2) | `projects/foundation/docs/rng.md`, foundation `be084c4` | "Lazlo's own repo; constants from public-domain reference C" | not stated | `2.8.md` section 6 | plan row; local path |
| crash-sim's CG cases | n6's `pcg_matches_crash_sim_cases` | `state/next-batch-2026-11-20/n20-05-numerics-sparse-cg.md` (outside Git) | "Lazlo's own" | not stated | `2.8.md` section 6 | plan row; local path |
| SuiteSparse Matrix Collection (optional, later) | larger real sparse matrices for a later brief; "Not in the first pass." | https://sparse.tamu.edu | "Per-matrix; to verify (D4)" | not stated | `2.8.md` section 6 | plan row; optional |
| Published benchmarks or sample sets for timing | none: the Done-when is about matching outputs, not speed | "None." | not applicable | - | `2.8.md` section 6 | plan row: none used |

### Sandbox node (docs/nodes/sandbox.md section 6)

The plan: "Listing only: nothing is downloaded. Every fact below is from memory and is UNVERIFIED until X1 reads the page and tries it."

| Data set | Used for | Source | Licence as stated | Size | Plan and section | Status |
|---|---|---|---|---|---|---|
| AppContainer isolation: objects, files, network, capabilities | design facts | https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation | "Microsoft Learn text; API facts only" | not stated | `sandbox.md` section 6 | plan row |
| Launching a process in an AppContainer (`SECURITY_CAPABILITIES`) | API facts | https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer | "same" | not stated | `sandbox.md` section 6 | plan row |
| `DeriveAppContainerSidFromAppContainerName`, `CreateAppContainerProfile` | API facts | https://learn.microsoft.com/en-us/windows/win32/api/userenv/ | "same" | not stated | `sandbox.md` section 6 | plan row |
| Job objects and their limits | API facts | https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects | "same" | not stated | `sandbox.md` section 6 | plan row |
| `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` | API facts | https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_cpu_rate_control_information | "same" | not stated | `sandbox.md` section 6 | plan row |
| `UpdateProcThreadAttribute` (handle list, child-process policy, security capabilities) | API facts | https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute | "same" | not stated | `sandbox.md` section 6 | plan row |
| AppContainer loopback isolation and its exemption | API facts | https://learn.microsoft.com/en-us/previous-versions/windows/apps/hh780593(v=win.10) | "same" | not stated | `sandbox.md` section 6 | plan row |
| Restricted tokens (the fallback) | API facts | https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens | "same" | not stated | `sandbox.md` section 6 | plan row |
| Chromium's Windows sandbox design (broker and target, job, token, AppContainer) | design reference | https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md | "BSD-style; design ideas only, no code read for copying" | not stated | `sandbox.md` section 6 | plan row |
| Sample sets, published escape suites | none: the probes are this repo's own, and the controls (x22) show each one can detect an escape | "None." | not applicable | - | `sandbox.md` section 6 | plan row: none used |

### Roadmap rows with no plan yet

These pass-1 rows are kept unchanged in substance: no merged plan has a section 6 for their node, so none replaces them. Their licences, URLs and sizes are still written from memory and **unverified** (pass 1). Status for each: **no plan yet**.

| Data set | Where to get it (to verify) | Licence (pass 1, unverified) | Rough size | Nodes (CAPABILITY_ROADMAP.md) | Redistributable in a repo? | Status |
|---|---|---|---|---|---|---|
| Khronos glTF Sample Assets (models plus reference renders) | github.com/KhronosGroup/glTF-Sample-Assets | verify before use: **Mixed per model** (CC0, CC-BY, CC-BY-NC and others). Decision for Lazlo: choose models by licence. | hundreds of MB for the full set; a chosen subset is far smaller | 2.5 real-time renderer; 2.4 glTF import/export | Only per model, and not the NC ones; keep out of Git, fetch on demand | no plan yet (UNVERIFIED, from memory) |
| Ghia, Ghia and Shin (1982) lid-driven cavity tables | The paper (J. Comput. Phys. 48); the tables are small numeric data | verify before use: Published numbers; the paper is copyrighted. Typing the numbers into a test is usual practice but is **a decision for Lazlo**. | under 10 KB | 3.20 CFD | Probably the numbers only, with citation; unclear | no plan yet (UNVERIFIED, from memory) |
| Cornell box scene and reference image | Cornell University Program of Computer Graphics site | verify before use: Unclear (data is widely copied, no clear licence). Decision for Lazlo; can be rebuilt from the published dimensions instead. | under 5 MB | 3.5 path tracer | Rebuild from dimensions rather than copy | no plan yet (UNVERIFIED, from memory) |
| Dam-break benchmark data | Published experiments (for example Martin and Moyce 1952) or a generated reference | verify before use: Paper numbers: same question as Ghia. | small | 3.3 fluids | Numbers only, with citation; unclear | no plan yet (UNVERIFIED, from memory) |
| Published COMSOL thermal-expansion tutorial result | COMSOL tutorial documentation | verify before use: Vendor copyright. **Decision for Lazlo**; prefer an analytic or open reference problem. | small | 3.21 multiphysics | Unclear; avoid copying | no plan yet (UNVERIFIED, from memory) |
| Test image for colour management matching | To be chosen (an open one, or generated) | verify before use: Unknown until chosen | small | 1.11 colour management | Generate our own: yes | no plan yet (UNVERIFIED, from memory) |
| IIHS crash test reports (named in DEVELOPMENT_PLAN stage 4 step 4 as an alternative to NHTSA) | iihs.org | verify before use: IIHS copyright, terms not checked. Decision for Lazlo. | PDFs only | Crash sim validation | No, unless licence allows | no plan yet (UNVERIFIED, from memory) |

Pass-1 rows replaced in pass 2 (removed, because a plan's section 6 replaces them): "SVG conformance test subset (W3C SVG test suite)", replaced by the node 1.3 rows; "LAPACK/SciPy reference outputs for numerics", replaced by the node 2.8 rows (the 2.8 plan asks for exactly this).

### Spot-check (pass 2)

Five rows with a URL or a named local path, checked against the plan text. No network was used: the URL checks compare the row to the plan's wording only (they do not show the URL is live), and the local-path checks use `Test-Path` against the main checkout at `C:\Users\MrComputer\Desktop\Broadsword` (the worktree does not contain `projects/`, `reference/` or `state/`, which are outside Git). Plan text was matched with exact-string search on the plan file.

| # | Row checked | Claim in the doc | Check made | Result |
|---|---|---|---|---|
| 1 | Node 1.1, `POINTER_PEN_INFO` | URL `https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info`; values "pressure 0-1024, tilt X and Y -90 to 90, rotation 0-359"; licence as in the plan | Exact-string search of `docs/nodes/1.1.md` for the URL and for the value text | PASS: both strings found in `1.1.md` section 6. The URL was not opened (no network); it is the plan's unverified memory URL. |
| 2 | Node 1.3, `projects/render/src` and `tests/golden.rs` | local path, licence "Own code." | Exact-string search of `docs/nodes/1.3.md`; `Test-Path` on `projects\render\src` and `projects\render\tests\golden.rs` in the main checkout | PASS: plan text found (both strings); both paths exist (`True`, `True`). |
| 3 | Node 2.8, foundation generator | path `projects/foundation/docs/rng.md`, commit `be084c4`, licence "Lazlo's own repo; constants from public-domain reference C" | Exact-string search of `docs/nodes/2.8.md`; `Test-Path` on the file; `git cat-file -t be084c4` inside `projects/foundation`; read `rng.md` for the public-domain claim | PASS: plan text found; file exists; `be084c4` is a `commit`; `rng.md` says both generators "are ports of the public-domain reference C code by Blackman and Vigna". |
| 4 | Node 2.8, crash-sim CG cases | path `state/next-batch-2026-11-20/n20-05-numerics-sparse-cg.md (outside Git)`, "Lazlo's own" | Exact-string search of `docs/nodes/2.8.md`; `Test-Path`; read the first lines | PASS: plan text found; file exists (`True`); first line is "# Brief: Sparse matrices and a preconditioned conjugate-gradient solve inside crash-sim". |
| 5 | Node 1.4, Noto fonts | URLs `https://github.com/notofonts` and `https://fonts.google.com/noto`; licence "SIL Open Font License 1.1 (https://openfontlicense.org). Redistribution in the repo needs the licence text kept with the files." | Exact-string search of `docs/nodes/1.4.md` for the URLs and the whole licence sentence | PASS: both found verbatim in `1.4.md` section 6. Not opened online; licence is the plan's from-memory statement ("Licences are from memory and are confirmed when the file is first used"). |

Not covered by the spot-check: no row's URL was fetched, so no licence in Part B is confirmed against its live source; that stays open (see Follow-up needed).

## Decisions for Lazlo (every non-free or unclear licence)

Items 1 to 7 are from pass 1 and are unchanged except where noted. Items 8 to 14 are new in pass 2 and come from the plans' section 6 text.

1. **CCSA Camry models (2012 v5a, 2015 BM v1) and their PDFs:** no stated licence. The manifest says keep them out of the repo and anything published. Question: is local-only research use enough for the crash sim, and may any derived mesh or reduced model be committed?
2. **Toyota owner's manual:** copyright. Local reference only, never committed.
3. **NHTSA tests and DOT HS 812 635:** US Government material, probably public domain, but the terms were not read here. Decide whether the small signal files may go into the repo (they are 0.4 to 10 MB each) or stay local only. The `dot.report` photos are a third-party site; treat as unclear.
4. **Khronos glTF samples (no plan yet):** per-model licences; choose the subset and drop CC-BY-NC models.
5. **Ghia, Martin and Moyce, COMSOL, Cornell box, IIHS (no plan yet):** copyrighted sources of small numeric data; decide whether quoting the numbers with a citation is acceptable.
6. **W3C SVG test suite (now node 1.3 rows):** the plan says "W3C test-suite licence; whether the files may be committed to a repo is **to confirm**." Decide which suite subset to use once its licence is read, and whether the files may be committed or must stay outside Git.
7. **Colour-management test image (no plan yet):** licence unknown until an image is chosen. Decision: generate our own image so no licence is needed, or name an open image and check its licence.
8. **resvg test suite (1.3, second candidate for the SVG conformance suite):** the plan says "To confirm" and does not state the licence. Do not copy any of it until its licence is read.
9. **Unicode conformance files (1.4: `BidiCharacterTest.txt`, `BidiTest.txt`, `GraphemeBreakTest.txt`, `LineBreakTest.txt`):** the plan says "Unicode terms of use (data files; copyright and permission notice kept in the fixture header)". Question: is committing these files with the notice acceptable, or should they be fetched at test time and kept out of Git? The terms were not read here.
10. **Noto fonts (1.4):** SIL Open Font License 1.1 per the plan, "Redistribution in the repo needs the licence text kept with the files." Question: vendor them in the repo with the licence text, or keep them outside Git?
11. **HarfBuzz and rustybuzz shaping tests (1.4):** MIT-style and MIT per the plan, used "only for fonts that are also licensed to vendor". Confirm the licence text of each test file before copying expected output.
12. **Wikipedia-sourced facts (1.7 Kahn's algorithm and LCG constants, 1.8 NTSC, 2.8 Bluestein):** the plans say CC BY-SA 4.0, "facts only, no text copied". Rule to confirm: only numbers and formulas are used, so no share-alike text enters the repo; the two LCG constants in 1.7 are "from memory: confirm against the page before use".
13. **Books and papers (2.8: Higham, Demmel and Veselic, Dormand and Prince, Hairer and Wanner, Hairer-Lubich-Wanner, Moré-Garbow-Hillstrom, Griewank and Walther; also LAPACK Users' Guide; 1.7 "Build Systems a la Carte"; 1.8 Premiere Pro tick figure, AES5):** copyright; the plans use them read-only, or for facts such as coefficients, equations and single numbers. Decide whether quoting those facts with a citation is acceptable (same question as item 5).
14. **MINPACK (2.8) and SuiteSparse Matrix Collection (2.8, optional, later):** the plan says "MINPACK licence (BSD-like, to verify)" and SuiteSparse "Per-matrix; to verify (D4)". Neither may be linked or downloaded until the licence is read; SuiteSparse is not in the first pass.

Also noted, no decision needed: Dear ImGui is MIT and Blender is GPL (1.5); Chromium's sandbox design is "BSD-style" (sandbox plan); Blender manual text is CC BY-SA 4.0 (1.7, 1.8); in each case the plan says only ideas are read and no code or text is copied.

## Disk planning

- Part A is about 320 MB, already on disk, and outside every repo.
- Part B downloads: no plan's section 6 states a size for any dataset. The plans download nothing; the only candidates with real size are the Unicode test files (small text files, size not stated), the Noto fonts (size not stated), the SVG and resvg test suites (size not stated, 1.3) and the full Khronos sample set (hundreds of MB, no plan yet), the largest item.
- Check free space against `disk_floor_gb` in `broadsword.toml` before any download, and place downloads under `reference/` (outside Git) or `shared/`, never inside a worktree.

## Follow-up needed

- **Every URL and licence in Part B is still unverified.** The plans themselves say they were written from memory with no network (1.1, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 2.8, sandbox). The brief that first relies on a row must open the page, confirm it says what the plan quotes, and correct this file. Pass 2 was told to use no network, so none was checked.
- **Rows with no plan yet:** Khronos glTF samples (nodes 2.5, 2.4), Ghia (3.20), Cornell box (3.5), dam-break (3.3), COMSOL (3.21), colour-management image (1.11), IIHS. Re-run this file when those plans merge.
- **Plans not in this pass:** `0.1.md` has no section 6. Any node plan merged after this pass (anything in `docs/nodes/` other than `0.1`, `1.1` to `1.8`, `2.8`, `sandbox`) is not covered.
- **Plan quirks to fix in the plans, not here:** `2.8.md` section 6 still says this file has one UNVERIFIED row for the node and that pass 2 should replace it; that is now done (the row is replaced by the 2.8 rows).
- **Part A hash note:** `MANIFEST.md` still says it lists 6 files (out of date, from pass 1); not changed here because it lies outside this task's claim.
