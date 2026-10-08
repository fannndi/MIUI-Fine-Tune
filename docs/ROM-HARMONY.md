# ROM harmony — MiFineTune ↔ MIUI 12 (surya, Android 10) ↔ kernel `surya-q-oss`

Kontrak kepemilikan untuk POCO X3 NFC. Ditulis dari tiga bukti:

1. **Device live** — `V12.0.7.0.QJGIDXM` (audit node runtime via `miui-ft probe`)
2. **ROM unpacked** — `miui_SURYAGlobal_V12.0.9.0.QJGMIXM_7f83537667_10.0`
   (MIO-KITCHEN) — diffa dengan device
3. **Kernel source** — `Xiaomi_Kernel_OpenSource` branch `surya-q-oss` (4.14 qcom)

## Siapa menulis apa

Katalog v0.4: **84 node** (49 Baseline, 35 Free).

| Node / parameter | Boot (`init.qcom.post_boot.sh`) | Runtime | Pemilik | MiFineTune |
|---|---|---|---|---|
| `scaling_governor`, `scaling_min/max_freq`, `schedutil/*` | ✓ blok `"365"\|"366"` | perf HAL (boost transient, via `msm_performance` lock — **bukan** node freq langsung) | ROM + perf HAL | **Baseline** (boleh disetel sebagai default) |
| `core_ctl/*` | ✓ (min/busy/task/not_preferred/offline_delay) | – | ROM | **Baseline** |
| `sched_up/downmigrate`, `group_*`, `walt_rotate`, `coloc_fmin` | ✓ | – | ROM | **Baseline** |
| `sched_latency/min_granularity/wakeup_granularity` | ✗ | ✗ | – | **Free** |
| `sched_min_task_util_for_boost` (51) / `for_colocation` (35) | ✓ colocation-v3 block (game-boost-off menulis 0/0 **transien**) | ✗ di surya (XML hanya mapping opcode 0x1E/0x1F; tak ada config msmsteppe yang memakainya) | ROM | **Baseline** (bounds 0..=1000, `sysctl.c` extra1=&zero/extra2=&one_thousand) |
| `sched_many_wakeup_threshold` (1000) | ✗ | (mapping XML minor 0x27; **tidak ada config** yang memakainya di msmsteppe) | – | **Baseline**-conservative (bounds 2..=1000, `extra1=&two`) |
| `sched_sync_hint_enable` (1) | ✗ | (mapping XML minor 0x28; tidak dipakai config) | – | **Baseline**-conservative (domain 0..=1) |
| `sched_time_avg_ms` (1000) | ✗ | ✗ | – | **Free** (>= 1, `proc_dointvec_minmax extra1=&one`) |
| `timer_migration` (1) | ✗ — post_boot menulis `power_aware_timer_migration` yang **tidak ada** di kernel ini (dead write, 4×) | ✗ | – | **Free** (0..=1, `timer_migration_handler`) |
| `sched_rr_timeslice_ms` (100) | ✗ | ✗ | – | **Free** (1..=1000; `sched_rr_handler` meng-reset ke default kalau ≤0) |
| `sched_tunable_scaling` / `sched_conservative_pl` / `sched_cstate_aware` | `conservative_pl` ✓ (REV-branch) | ✗ | ROM | **Free** (masing-masing terbanding; `conservative_pl` Baseline 0..=1) |
| `sched_lib_name` / `sched_lib_mask_force` | ✗ | ✓ **perfd** (strings libqti-perfd; komentar kernel: "perfd already configure sched_lib_mask_force to 0xf0") | perf HAL | **Forbidden** + `sched_lib_mask_check` (node absen) |
| `queue/iosched/*` (cfq: `quantum/slice_idle_us/target_latency_us/…`) | ✗ (hanya `read_ahead_kb`) | ✗ | – | **Free** — **per-scheduler**: dir hanya ada saat cfq aktif (pass-2 seperti schedutil); game (deadline) tidak menyentuhnya |
| `sched_migration_cost_ns` | ✗ | ✓ perf HAL (`commonresourceconfigs.xml` Opcode 0x2) | perf HAL | **Baseline** (temuan audit tool) |
| `sched_boost` | ✓ (0 di akhir blok) | ✓ perf HAL | perf HAL | **Forbidden** |
| `vm.*` kecuali swappiness/min_free/page-cluster/watermark_scale | ✗ | ✗ | – | **Free** |
| `vm.swappiness`, `min_free_kbytes`, `page-cluster` | ✓ (`configure_memory_parameters`) | lmkd | ROM | **Forbidden** |
| `vm.watermark_scale_factor` | ✓ semua target di-set `1` ("we are using efk"; ROM sendiri menulis rentang 1..1000) | – | ROM | **Baseline** (temuan audit tool) |
| `net.tcp_rmem/wmem` | ✗ | ✓ **network stack**: ConnectivityService mengirim `LinkProperties.TcpBufferSizes` (nilai carrier, terlihat di `dumpsys connectivity`) ke netd → tulis procfs; ter-reset di siklus display-off | framework | **Baseline** (tidak dipakai profile; temuan empiris 2026-10-07) |
| `net.*` lainnya (cc/fin_timeout/fastopen/mtu_probing/slow_start) | ✗ | ✗ | – | **Free** |
| `net.core/*` (rmem_max, netdev_max_backlog, …) | ✗ | (stack-adjacent; tidak diaudit per node) | framework-ish | **Tidak dikatalog** (aman = tidak disentuh) |
| `io.scheduler` / `nr_requests` / `nomerges` / `iostats` / `rq_affinity` | ✗ (hanya `read_ahead_kb` yang ditulis) | ✗ | – | **Free** |
| `stune/*/schedtune.*` | ✓ `top-app/prefer_idle` | ✓ perf HAL `top-app` | ROM + perf HAL | **Baseline** |
| `stune/{rt,audio-app}` | ✗ | ✓ audio HAL / RT task framework | framework | **Forbidden** (cgroup framework) |
| `stune.root/…/schedtune.colocate` | ✓ `init.target.rc` (root/bg/sys-bg/fg=0, top-app=1) | ✗ | ROM | **Baseline** (tidak direferensikan profile; coexist) |
| cpuset `background/system-background/foreground/top-app` | ✓ (bg/system-bg) + `writepid` | ✓ perf HAL + framework API | ROM + PowerKeeper | **Baseline** |
| cpuset `game/gamelite/vr` | ✗ | ✓ PowerKeeper (game mode) | PowerKeeper | **Forbidden** |
| cpuset `audio-app/camera-daemon/restricted` | ✗ (mkdir `init.target.rc`, uid cameraserver) | ✓ cameraserver / audio framework | framework | **Forbidden** |
| GPU `min/max/default_pwrlevel` + `devfreq/min|max_freq` | ✗ | ✓ perf HAL + **thermal cooling** (`thermal-devfreq-0`) | perf HAL + thermal | **Baseline** (hati-hati: dua tampilan limiter sama — lihat "Drift" di bawah) |
| `gpu.devfreq/min_freq` & `max_freq` (Hz view) | ✗ | ✓ perf HAL (xml:gpu) | perf HAL | **Forbidden** (exact-path guard; view Hz milik framework) |
| `gpu.devfreq/governor` | ✗ | ✗ | – | **Baseline** — realita device: hanya `msm-adreno-tz` diterima kgsl |
| `thermal_message/*`, cooling devices, `msm_performance/*`, `cpu_boost/*`, charge, zRAM | ✗ | ✓ mi_thermald / perf HAL / micharge | framework | **Forbidden** (guard path) |
| perf HAL runtime-only (`/dev/cpuset/foreground/boost/cpus`, `/dev/cpu_dma_latency`, `/sys/kernel/mm/ksm/*`, kgsl `force_no_nap/clk_on/rail_on/idle_timer`, `mmc0/clk_scaling`, `proc_reclaim`, `swap_ratio`, `/proc/%d/sched_group_id`) | ✗ | ✓ libqti-perfd (OptsHandler) / PowerKeeper | framework | **Forbidden** |
| `workqueue.power_efficient` | – | – | **kernel** (0444 hardcoded) | **Tidak pernah dikatalog** (`kernel/workqueue.c:294`) |

**Identitas lintas-versi (terbukti):** `post_boot.sh`, kelima `vendor/etc/perf/*.xml`,
`powerhint.xml`, dan `thermal-{normal,map,tgame}.conf` **identik byte-per-byte**
antara `V12.0.7.0.QJGIDXM` (device) dan `V12.0.9.0.QJGMIXM`. Owner Map berlaku
untuk keluarga surya-Q MIUI 12.

**Q tidak punya `millet_monitor`** (freeze = framework API PowerKeeper) dan
tidak punya `cmd game` (Game Mode API MIUI 14 tidak ada di sini).

**Temuan audit 2026-10-07 (v0.3):**

- **Perf HAL punya event display off/on sendiri** (`perfboostsconfig.xml` Id
  `0x1040`/`0x1041` → opcode `0x40000000`; grup `display off` di
  commonresourceconfigs). **Uji empiris dengan `tools/display-off-diff.sh`**
  (stock, 60 dtk): satu-satunya node katalog yang berubah saat layar mati adalah
  `net.tcp_rmem/wmem` (reset oleh network stack, lihat baris tabel) — tidak ada
  node katalog lain yang disentuh; MIUI tidak memarkir frekuensi pada display-off
  selain jalur thermal biasa.
- **Daftar runtime writer definitif** ada di `tools/perf-hal-runtime-writers.txt`
  (strings `libqti-perfd.so` + XML major groups + `netd`); audit tool v2 gagal
  bila ada node tier Free yang bertabrakan dengannya.
- **`schedutil/*` runtime**: XML perf HAL menunjuk path legacy
  `/sys/devices/system/cpu/cpufreq/schedutil/*` yang **tidak ada di surya**
  (hanya `policy0/policy6`) → tulisan tersebut gagal senyap; tunable schedutil
  kita murni Baseline dari post_boot.
- **Dead code terverifikasi**: `pm2/idle_sleep_mode` hanya cabang target msm7630
  kuno; `app_setting` + `sched_lib_*` (ditulis perf HAL) tidak ada di kernel
  OSS maupun device → tidak ada yang perlu diharmonikan di sana.
- **Thermal clamp pada cap freq (kind `FreqMax`)**: `scaling_max_freq` dianggap
  **in-sync bila live ≤ permintaan** — cap eksternal yang lebih ketat (thermal
  cooling / freq-QoS) = thermal menang, sesuai filosofi harmoni. Live > permintaan
  (cap hilang) baru dianggap drift dan dipulihkan guard. Sebelum semantik ini
  (exact-match), apply/restore melaporkan failure palsu saat thermal aktif —
  kejadian nyata 2026-10-07: gold ter-hold `1209600` vs `1555200` yang diminta.
- **QoS floor pada floor freq (kind `FreqMin`, v0.4)**: simetrisnya — perf HAL
  menahan floor `scaling_min_freq` **lebih tinggi** via QoS `msm_performance`
  saat game boost; live ≥ permintaan = in-sync (framework menang), live <
  permintaan = drift. Eksperimen live: tulis `576000` saat QoS `1248000` → baca
  `1248000` (sebelumnya = `read-back mismatch` palsu yang memicu retry di
  watcher); QoS lepas → node kembali sendiri ke nilai yang ditulis.
- **`io.scheduler` read-back = hanya token bracketed yang aktif**: daftar
  offered `noop deadline [cfq]` — bug v0.3 terverifikasi: `deadline` cocok
  sebagai token offered padahal cfq aktif → apply game **tidak pernah benar-
  benar menulis elevator**. Diperbaiki (hanya `[deadline]` yang dihitung) +
  uji device: apply `dltest` sekarang benar-benar memindahkan elevator dan
  dir `iosched` berganti isi (cfq↔deadline tunables).
- **`power_aware_timer_migration` = dead write**: post_boot menulisnya 4×,
  tapi node tidak ada di kernel surya 4.14 (kelas temuan sama dengan
  `workqueue.power_efficient`); `timer_migration` standar yang ada = Free.
- **perfd WALT lib-hint**: `sched_lib_name`/`sched_lib_mask_force` runtime-
  owned oleh perfd (strings libqti-perfd + komentar kernel
  drivers/cpufreq/cpufreq.c "perfd already configure sched_lib_mask_force to
  0xf0") → **Forbidden**; `sched_lib_mask_check` tidak ada di device.
- **Eksplorasi XML minor opcodes (v0.4)**: commonresourceconfigs.xml memetakan
  28 minor opcode → node `sched_*` (0x19 initial_task_util, 0x21 user_hint,
  0x26 window_stats_policy, 0x29 ravg_window_nr_ticks, dst) — SEMUA node
  tersebut **tidak ada** di kernel surya 4.14 (mapping untuk target lain);
  yang ada dan dikatalog: `min_task_util_for_boost/colocation`,
  `many_wakeup_threshold`, `sync_hint_enable`. `perfboostsconfig.xml` di surya
  hanya memakai **major opcodes** (cpufreq/gpu QoS) — sched minors tidak pernah
  ditulis runtime di device ini.
- **cgroups framework (v0.4)**: `cpuset {audio-app,camera-daemon,restricted}`
  dan `stune {rt,audio-app}` dikelola init/cameraserver/audio HAL → Forbidden.
  `stune.*.schedtune.colocate` boot-written oleh `init.target.rc` (top-app=1)
  → Baseline, tidak direferensikan profile.

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
| `scaling_max_freq` (cap) | thermal cooling/freq-QoS dapat memegang cap **lebih rendah** dari permintaan — itu bukan error | kind `FreqMax`: live ≤ want = in-sync; live > want = drift |
| `scaling_min_freq` (floor) | **simetris**: QoS `msm_performance` (`perf_adjust_notify` → `CPUFREQ_ADJUST` → `cpufreq_verify_within_limits`, `drivers/soc/qcom/msm_performance.c:256`) memegang floor **lebih tinggi** — bukan error | kind `FreqMin`: live ≥ want = in-sync; live < want = drift. Eksperimen live 2026-10-07: tulis 576000 saat QoS 1248000 → baca 1248000; QoS lepas → kembali sendiri ke 576000 |
| `sched_many_wakeup_threshold` | `extra1=&two` (`kernel/sysctl.c`) — kernel menolak < 2 | rentang 2..=1000 |
| `sched_min_task_util_for_boost/colocation` | `extra1=&zero, extra2=&one_thousand` (`kernel/sysctl.c:389`) | rentang 0..=1000 |
| `sched_rr_timeslice_ms` | `sched_rr_handler` (`kernel/sched/rt.c:2912`): tulis ≤ 0 = **reset ke default** | rentang 1..=1000 agar profil tidak pernah bermakna "reset" |
| `queue/iosched/*` | dir dibuat/dihancurkan bersama elevator aktif (`cfq_init_queue` / `deadline_init_queue`) | pass-2 re-plan saat "node missing" (sama dengan `schedutil`) |
| `io.scheduler` read-back | `show_one` = daftar offered dengan token aktif **bracketed** | kind `IoSched`: hanya token bracketed yang dianggap aktif — **bug v0.3 terverifikasi**: token offered unbracketed salah-positif → apply `deadline` pernah di-skip selamanya |

## Urutan tulis (keamanan)

`vm → net → kernel → io → gpu → cpuset → stune → core_ctl → schedutil →
governor → scaling_max → scaling_min` — freq max sebelum min; pasangan kernel
di-reorder sesuai aturan di atas. Restore memakai urutan yang sama.

## Drift (siapa yang menimpa setelah apply)

- **Uji empiris (touch boost ×12 + 5 dtk):0 drift** — semua key Power Save
  bertahan (gpu cap, cpuset, stune, migration_cost, scaling_max).
- **Display-off (v0.4, katalog 84): 0 dari 84 node berubah** — cfq tunables,
  sched colocation family, timer_migration, rr_timeslice tidak disentuh
  framework saat layar mati.
- Karena skenario *app-launch/game boost* belum teruji, app menjalankan
  **drift guard**: verify read-only tiap 15 dtk saat profile aktif → re-apply
  **hanya key yang drift** → terlihat di status card ("Drift guard").
- Risiko terkonfirmasi (runtime writer list): perf HAL boost menulis
  `devfreq/min|max_freq` (Hz) — tampilan lain dari `min|max_pwrlevel` (mapping
  di `kgsl_pwrscale`/`kgsl_pwrctrl.c:839-865`) — plus `core_ctl` lock
  min/max_cores, cpusets, stune, dan QoS `msm_performance`; guard menutup
  semuanya (20 entri Baseline overlap tercetak oleh audit tool v2).
- Siklus display-off (stock): network stack me-reset `net.tcp_rmem/wmem`;
  profile tidak lagi menyentuhnya supaya tidak melawan framework.

## Mode bawaan MIUI (bridge v0.4.2, eksekusi 2026-10-08)

| Mode MIUI | State asli | Tulis aplikasi | Keterangan |
|---|---|---|---|
| Battery saver | `Settings.Global low_power` | root put — **live** ✓ | page MIUI mengikuti flag; restore ke snapshot via `MiBridgeState` |
| Performance (hidden sheet) | `persist.sys.aries.power_profile` | **tidak bisa** — SELinux menolak setprop dari semua ctx su (shell/run-as/untrusted_app); dialog tersembunyi tak bisa dibuka di atas game terkunci | yang ditulis = mirror `Settings.System power_mode` saja (silent); label switch menyebut keterbatasan |
| Ultra battery saver | broadcast `EXTREME_POWER_SAVE_MODE_CHANGED` (tidak persisten) | retire: restore + stop + config off | MIUI membekukan service kita sendiri — nol intervensi = tujuan tercapai |
| Split screen / floating window | `GameBoosterService` log `mMultiWindowForegroundPackageName` != 'null' | deteksi via stream logcat -b main (RootBridge) | arbiter memaksa `Balance` menang atas mapping & saver (keputusan user); layar mati tetap lebih tinggi |
| Game Booster (checker) | `thermal_message/sconfig != 0` | root cat | notifikasi konflik sekali per sesi game |

Pola tulis/snapshot mode = state machine murni `MiBridgeState`
(hold → capture nilai user → release → restore; attribution: tulisan kita
tidak dianggap pilihan user oleh arbiter). Prefs menyimpan hold agar death
mid-hold tetap bisa dipulihkan di service berikutnya.

Catatan deteksi transisi: dva tag event `am_resume_activity` dan
`am_set_resumed_activity` (beberapa jalur launch — monkey/new task — hanya
mengeluarkan yang kedua). Seed `peekEvents` hanya mempercayai event segar
(≤60 dtk) karena buffer events memuat berjam-jam sejarah.

## Verifikasi

```bash
# audit Owner Map terhadap ROM unpacked mana pun (jalankan tiap ganti ROM/kernel)
tools/owner-map-audit.sh <unpacked-rom-dir>

# uji empiris perilaku layar-mati (baca 84 node + node framework, matikan
# layar lewat power-key, diff otomatis)
tools/display-off-diff.sh 60

# di device
su -c /data/local/tmp/mifinetune/miui-ft probe --json   # baca semua node
su -c /data/local/tmp/mifinetune/miui-ft verify <id>    # deteksi drift
su -c /data/local/tmp/mifinetune/miui-ft restore        # kembali stock
```

**Aturan lama yang tetap berlaku**: engine menulis baseline, bukan merebut
kepemilikan; thermal selalu menang di `scaling_max`; restore mengembalikan
snapshot 100% (termasuk quirk `hispeed 1324600` yang bukan OPP).
