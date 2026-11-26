# Data and licences

Nothing large is committed. `cargo run --release -- fetch` downloads each photo from Wikimedia
Commons and refuses it unless its size and SHA-1 match what Commons lists for the file.

## The photos

Twenty photos of damaged cars, curated from Commons' categories *Automobiles with minor crash
damage*, *with moderate crash damage* and *with major crash damage*. Every file is under a free
licence that allows reuse with attribution; none is non-commercial or no-derivatives. Photos
with people in them were left out, and so were vehicles whose make could not be told from the
photo. The severity each claim is scored against is the category Commons editors filed the
photo in.

| Key | File on Commons | Author | Licence | Commons category severity | SHA-1 |
|-----|-----------------|--------|---------|---------------------------|-------|
| `suzuki1` | [Abandoned Suzuki, Osidge, London.jpg](https://commons.wikimedia.org/wiki/File:Abandoned_Suzuki,_Osidge,_London.jpg) | Philafrenzy | CC BY-SA 4.0 | minor | `1b8c6c9bd44399e66080c37c1286a89ccd7cf75e` |
| `suzuki2` | [Abandoned Suzuki, Osidge, London (2).jpg](https://commons.wikimedia.org/wiki/File:Abandoned_Suzuki,_Osidge,_London_(2).jpg) | Philafrenzy | CC BY-SA 4.0 | minor | `e5b31cc50d2f20b54f0fef75398611fcec299803` |
| `dodge` | [US Government wrecked Dodge Stratus in Manhattan, 2005.jpg](https://commons.wikimedia.org/wiki/File:US_Government_wrecked_Dodge_Stratus_in_Manhattan,_2005.jpg) | Diego Torres Silvestre from Sao Paulo, Brazil | CC BY 2.0 | minor | `7667a3594d17b8160a76a3f19d9316a4e4e89425` |
| `hhr` | [2007 Chevrolet HHR involved in crash.jpg](https://commons.wikimedia.org/wiki/File:2007_Chevrolet_HHR_involved_in_crash.jpg) | Crazytales | CC0 | moderate | `2aa68ff03f3090a94457e57e79e7e454f849807c` |
| `civic` | [90e Avenue, LaSalle - 10.jpg](https://commons.wikimedia.org/wiki/File:90e_Avenue,_LaSalle_-_10.jpg) | Jeangagnon | CC BY-SA 4.0 | moderate | `aa093dca264cf7a7c882d9e429e975b4a2e738ca` |
| `sonic` | [90e Avenue, LaSalle - 12.jpg](https://commons.wikimedia.org/wiki/File:90e_Avenue,_LaSalle_-_12.jpg) | Jeangagnon | CC BY-SA 4.0 | moderate | `e6ab08feb23d8b7b19d51c426ea004b68beb8ecb` |
| `w211` | [Crashed black W211.jpg](https://commons.wikimedia.org/wiki/File:Crashed_black_W211.jpg) | Matti Blume | CC BY-SA 3.0 | moderate | `e22ba45120e14b1c0a2c31641d73154fc35d3327` |
| `fabia` | [Fabia I (2003) accident front right.jpg](https://commons.wikimedia.org/wiki/File:Fabia_I_(2003)_accident_front_right.jpg) | Hundehalter | CC BY-SA 4.0 | moderate | `4e2bb1423e3415be0bdcdf27ffbac5dbe987e0a6` |
| `slk` | [Front crashed SLK.jpg](https://commons.wikimedia.org/wiki/File:Front_crashed_SLK.jpg) | Matti Blume (MB-one) | CC BY-SA 3.0 | moderate | `2a6470026450a7d0462fa5876e1fe60f9f25e8be` |
| `c5` | [Poor Citroen C5... (44990379005).jpg](https://commons.wikimedia.org/wiki/File:Poor_Citroen_C5..._(44990379005).jpg) | Guillaume Vachey from Chalon sur Saone, France | CC0 | moderate | `4eb7ac29f6f50778e0ff90931cf6da003bff695b` |
| `levorg` | [Subaru Levorg (48028046188).jpg](https://commons.wikimedia.org/wiki/File:Subaru_Levorg_(48028046188).jpg) | Guillaume Vachey from Chalon sur Saone, France | CC0 | moderate | `d9548e6d2a5eafdeab69e007e740fbdbb5bfcbd4` |
| `sprinter` | [90e Avenue, LaSalle - 36.jpg](https://commons.wikimedia.org/wiki/File:90e_Avenue,_LaSalle_-_36.jpg) | Jeangagnon | CC BY-SA 4.0 | major | `3c1cb7cfa6c6da7d59f734b4be70d46093a7753a` |
| `kia` | [Broken Kia.jpg](https://commons.wikimedia.org/wiki/File:Broken_Kia.jpg) | Oleg Yunakov | CC BY-SA 4.0 | major | `d2a46647882d1ade50ee34f11714cc3d1890a242` |
| `sclass` | [Crashed black S-Class.jpg](https://commons.wikimedia.org/wiki/File:Crashed_black_S-Class.jpg) | Matti Blume (MB-one) | CC BY-SA 3.0 | major | `c7fbe7078a3228bde6fa3efda77d0769a2119895` |
| `audi` | [Damaged Audi, New Barnet (2).jpg](https://commons.wikimedia.org/wiki/File:Damaged_Audi,_New_Barnet_(2).jpg) | Philafrenzy | CC BY-SA 4.0 | major | `f7ac7e30094203a698e1cc7266eb42542139a893` |
| `lupo` | [Damaged Volkswagen Lupo in Jura, France (2018) 1.jpg](https://commons.wikimedia.org/wiki/File:Damaged_Volkswagen_Lupo_in_Jura,_France_(2018)_1.jpg) | Sebleouf | CC BY-SA 4.0 | major | `7e630d21fbaf2a816f171ac63bcfde7b1e14bb4b` |
| `c15` | [Citroen C 15 D 2001 (10431763586).jpg](https://commons.wikimedia.org/wiki/File:Citroen_C_15_D_2001_(10431763586).jpg) | order_242 from Chile | CC BY-SA 2.0 | major | `9f4f7777dfab4f9fb082cdc48949c63a62f76320` |
| `typer` | [Fn22.jpg](https://commons.wikimedia.org/wiki/File:Fn22.jpg) | Canelo 93 | CC BY-SA 4.0 | major | `148072f51a0f044b70f77c2fb691ffbf43f2cc03` |
| `impala` | [Vehicle After a Side-Impact Collision.jpg](https://commons.wikimedia.org/wiki/File:Vehicle_After_a_Side-Impact_Collision.jpg) | Kkiefuik | CC BY 4.0 | major | `6fcac6659c64e2343691410e8f7024266a03b807` |
| `cclass` | [Wrecked Mercedes, north Finchley, London 01.JPG](https://commons.wikimedia.org/wiki/File:Wrecked_Mercedes,_north_Finchley,_London_01.JPG) | Philafrenzy | CC BY-SA 4.0 | major | `1a212bbb27bfefe6b2c21fa3dfb9339c7a21690f` |

Attribution, as each licence asks: the author and licence above, and the file's Commons page
for its full licence text. Files under CC BY-SA are shown and scaled here, never redistributed
from this repository.

## What `fetch` builds from them

| File | What it is |
|------|------------|
| `data/photos/CL-0401-1.jpg` … | Each claim's photos, scaled to 1568 pixels on the long edge, the size Claude reads without scaling again |
| `CL-0418-1.jpg`, `CL-0419-1.jpg` | Second copies of earlier claims' photos (`hhr`, `w211`): cropped by 4% on each side and re-compressed, as a reused photo arrives |

The first notices of loss in `src/data.rs` are invented around the photos: policy numbers,
dates and what happened. `CL-0404` pairs two different cars photographed in the same yard
(`civic`, `sonic`), and `CL-0414` adds a van (`c15`) to a hatchback's claim (`lupo`): the two
"different vehicle" cases.

## Models

| Model | Where it runs | Licence | How it arrives |
|-------|---------------|---------|----------------|
| RF-DETR Nano (`rf-detr-n`) | This computer | Apache-2.0 | adk-vision's built-in manifest, fetched by revision and SHA-256 checked |
| SAM 2.1 Hiera Tiny, q4 (`sam2.1-hiera-tiny-q4`) | This computer | Apache-2.0 | The same |
| SigLIP 2 base, quantized (`siglip2-base-patch16-256-quantized`) | This computer | Apache-2.0 | The same |
| `claude-sonnet-5-5` | Anthropic's API | Anthropic's commercial terms | Called with your `ANTHROPIC_API_KEY` in a `--live` run |

## The recording

`recordings/tape.json` holds what Claude answered in the live run behind the README's results:
the agent's turns and the damage assessments, with their token counts. It holds no image. A run
without `--live` replays it.
