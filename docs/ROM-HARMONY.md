# ROM harmony — MiFineTune ↔ MIUI 12 (surya, Android 10) ↔ kernel `surya-q-oss`

Kontrak kepemilikan untuk POCO X3 NFC. Ditulis dari tiga bukti:

1. **Device live** — `V12.0.7.0.QJGIDXM` (audit node runtime via `miui-ft probe`)
2. **ROM unpacked** — `miui_SURYAGlobal_V12.0.9.0.QJGMIXM_7f83537667_10.0`
   (MIO-KITCHEN) — diffa dengan device
3. **Kernel source** — `Xiaomi_Kernel_OpenSource` branch `surya-q-oss` (4.14 qcom)

## Siapa menulis apa

| Node / parameter | Boot (`init.qcom.post_boot.sh`) | Runtime | Pemilik | MiFineTune |
|---|---|---|---|---|
| `scaling_governor`, `scaling_min/max_freq`, `schedutil/*` | ✓ blok `"365"\|"366"` | perf HAL (boost transient, via `msm_performance` lock — **bukan** node freq langsung) | ROM + perf HAL | **Baseline** (boleh disetel sebagai default) |
| `core_ctl/*` | ✓ (min/busy/task/not_preferred/offline_delay) | – | ROM | **Baseline** |
| `sched_up/downmigrate`, `group_*`, `walt_rotate`, `coloc_fmin` | ✓ | – | ROM | **Baseline** |
| `sched_latency/min_granularity/wakeup_granularity` | ✗ | ✗ | – | **Free** |
| `sched_migration_cost_ns` | ✗ | ✓ perf HAL (`commonresourceconfigs.xml` Opcode 0x2) | perf HAL | **Baseline** (temuan audit tool) |
| `sched_boost` | ✓ (0 di akhir blok) | ✓ perf HAL | perf HAL | **Forbidden** |
| `vm.*` kecuali swappiness/min_free/page-cluster/watermark_scale | ✗ | ✗ | – | **Free** |
| `vm.swappiness`, `min_free_kbytes`, `page-cluster` | ✓ (`configure_memory_parameters`) | lmkd | ROM | **Forbidden** |
| `vm.watermark_scale_factor` | ✓ semua target di-set `1` ("we are using efk"; ROM sendiri menulis rentang 1..1000) | – | ROM | **Baseline** (temuan audit tool) |
| `net.*` (tcp_rmem/wmem/cc/fin_timeout/fastopen/...) | ✗ | ✗ | – | **Free** |
| `io.scheduler` / `nr_requests` / `nomerges` / `iostats` / `rq_affinity` | ✗ (hanya `read_ahead_kb` yang ditulis) | ✗ | – | **Free** |
| `stune/*/schedtune.*` | ✓ `top-app/prefer_idle` | ✓ perf HAL `top-app` | ROM + perf HAL | **Baseline** |
| cpuset `background/system-background/foreground/top-app` | ✓ (bg/system-bg) + `writepid` | ✓ perf HAL + framework API | ROM + PowerKeeper | **Baseline** |
| cpuset `game/gamelite/vr` | ✗ | ✓ PowerKeeper (game mode) | PowerKeeper | **Forbidden** |
| GPU `min/max/default_pwrlevel` + `devfreq/min|max_freq` | ✗ | ✓ perf HAL + **thermal cooling** (`thermal-devfreq-0`) | perf HAL + thermal | **Baseline** (hati-hati: dua tampilan limiter sama — lihat "Drift" di bawah) |
| `gpu.devfreq/governor` | ✗ | ✗ | – | **Baseline** — realita device: hanya `msm-adreno-tz` diterima kgsl |
| `thermal_message/*`, cooling devices, `msm_performance/*`, `cpu_boost/*`, charge, zRAM | ✗ | ✓ mi_thermald / perf HAL / micharge | framework | **Forbidden** (guard path) |
| `workqueue.power_efficient` | – | – | **kernel** (0444 hardcoded) | **Tidak pernah dikatalog** (`kernel/workqueue.c:294`) |

**Identitas lintas-versi (terbukti):** `post_boot.sh`, kelima `vendor/etc/perf/*.xml`,
`powerhint.xml`, dan `thermal-{normal,map,tgame}.conf` **identik byte-per-byte**
antara `V12.0.7.0.QJGIDXM` (device) dan `V12.0.9.0.QJGMIXM`. Owner Map berlaku
untuk keluarga surya-Q MIUI 12.

**Q tidak punya `millet_monitor`** (freeze = framework API PowerKeeper) dan
tidak punya `cmd game` (Game Mode API MIUI 14 tidak ada di sini).

## Invariant kernel (dari source `surya-q-oss`, diverifikasi device)

| Node | Aturan source | Dampak ke validator |
|---|---|---|
| `kernel.sched_upmigrate` / `sched_downmigrate` | `sched_updown_migrate_handler` (`kernel/sched/core.c:6943`): tulis yang melanggar **di-rollback + `-EINVAL`** — `margin_up ≤ margin_down` ⇔ `upmigrate ≥ downmigrate` | pasangan divalidasi sebelum tulis + urutan tulis diturunkan dari nilai live (down dulu, kecuali `want_up > cur_down`) |
| `core_ctl/task_thres` | `store_task_thres` (`core_ctl.c:154`): `val < num_cpus → -EINVAL` | min = jumlah CPU cluster (silver ≥ 6, gold ≥ 2) |
| `core_ctl/min_cpus` | `store_min_cpus`: `min(val, max_cpus)` — **clamp senyap** | pre-clamp ke `max_cpus` live agar read-back cocok |
| `core_ctl/busy_*_thres` | 1 nilai = broadcast, atau tepat `num_cpus` nilai | kind `RepeatInt` (tulis1, verifikasi semua elemen sama) |
| `gpu.max_pwrlevel` | `kgsl_pwrctrl_max_pwrlevel_store` (`kgsl_pwrctrl.c:692`): `level > min_pwrlevel → level = min_pwrlevel` — **clamp senyap** | invariant `max ≤ min` (sama-sama boleh); read-back verify menangkap clamp (pernah terjadi saat dev: tulis 4 → baca 3) |
| `stune/*/boost` | `boost_write` (`tune.c:619`): `boost < 0 \|\| boost > 100 → -EINVAL` | rentang 0..=100 |
| `vm.watermark_scale_factor` | `extra1=&one, extra2=&one_thousand` (`kernel/sysctl.c:1648`) | rentang 1..=1000 |

## Urutan tulis (keamanan)

`vm → net → kernel → io → gpu → cpuset → stune → core_ctl → schedutil →
governor → scaling_max → scaling_min` — freq max sebelum min; pasangan kernel
di-reorder sesuai aturan di atas. Restore memakai urutan yang sama.

## Drift (siapa yang menimpa setelah apply)

- **Uji empiris (touch boost ×12 + 5 dtk):0 drift** — semua key Power Save
  bertahan (gpu cap, cpuset, stune, migration_cost, scaling_max).
- Karena skenario *app-launch/game boost* belum teruji, app menjalankan
  **drift guard**: verify read-only tiap 15 dtk saat profile aktif → re-apply
  **hanya key yang drift** → terlihat di status card ("Drift guard").
- Risiko teoritis: perf HAL boost menulis `devfreq/min|max_freq` (Hz) yang
  adalah tampilan lain dari `min|max_pwrlevel` (mapping di
  `kgsl_pwrscale`/`kgsl_pwrctrl.c:839-865`) → cap GPU bisa ter-reset oleh
  boost restore; guard menutup skenario ini.

## Verifikasi

```bash
# audit Owner Map terhadap ROM unpacked mana pun (jalankan tiap ganti ROM/kernel)
tools/owner-map-audit.sh <unpacked-rom-dir>

# di device
su -c /data/local/tmp/mifinetune/miui-ft probe --json   # baca semua node
su -c /data/local/tmp/mifinetune/miui-ft verify <id>    # deteksi drift
su -c /data/local/tmp/mifinetune/miui-ft restore        # kembali stock
```

**Aturan lama yang tetap berlaku**: engine menulis baseline, bukan merebut
kepemilikan; thermal selalu menang di `scaling_max`; restore mengembalikan
snapshot100% (termasuk quirk `hispeed 1324600` yang bukan OPP).
