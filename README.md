# MiFineTune

Profile switcher **selaras framework MIUI** untuk POCO X3 NFC (surya) — MIUI 12 /
Android 10. Tiga profile one-tap (**Power Save / Balance / Game**) yang **hanya
menyetel parameter bebas atau boot-baseline**, tidak pernah merebut kepemilikan
framework MIUI.

> Rust core (`miui-ft`) adalah satu-satunya penulis parameter. Aplikasi Kotlin
> (Material 3) hanya memanggil binary via `su` dan menampilkan laporan.

## Prinsip kepemilikan (Owner Map)

| Tier | Arti | Contoh |
|---|---|---|
| `free` | Tidak ditulis siapapun di ROM/runtime | `vm.*`, `net.*`, `kernel.sched_*` (di luar blok post_boot), `block/queue/*` |
| `baseline` | Ditulis **sekali saat boot** oleh `init.qcom.post_boot.sh` / transient oleh perf HAL — kita setel sebagai baseline, framework bebas menimpa | `scaling_governor`, `schedutil/*`, freq range, `core_ctl`, `stune`, cpuset non-game, GPU pwrlevel, I/O scheduler |
| **dilarang** | Runtime milik framework — **selalu ditolak validator** bahkan jika disebut profile | thermal (`mi_thermald`, `thermal_message`, cooling), `msm_performance`/`cpu_boost`, `cpuset game/gamelite/vr` (PowerKeeper), LMK/zRAM/swappiness, charge |

Aturan inti: **jangan rebut kepemilikan, boleh menyetel baseline.** Thermal
tetap menang di `scaling_max`, boost perf HAL tetap transient, game cpuset
tetap milik PowerKeeper.

## Arsitektur

```
app/   (Kotlin + Compose Material3)
├─ core/RootBridge.kt     deploy binary ke /data/local/tmp + eksekusi su
├─ core/FtClient.kt       klien JSON untuk miui-ft
├─ core/Models.kt         parser protocol
├─ ui/HomeViewModel.kt    state: status, plan per profile, apply/restore
└─ ui/HomeScreen.kt       status card + 3 kartu profile + dialog laporan
core/  (Rust — satu-satunya writer)
├─ catalog.rs   59 node: tier + path + kind + guard_path (anti-intervensi)
├─ probe.rs     pembacaan read-only + opsi validasi + bukti framework
├─ profile.rs   validasi & perencanaan (OPF clamp, invariant kernel, urutan tulis)
├─ apply.rs     snapshot → tulis → read-back verify → restore
└─ main.rs      probe | profiles | plan | apply | restore | verify | status
```

### Jaminan keamanan (teruji di device)

- **Snapshot**: nilai stock direkam sebelum tulis pertama; `restore`
  mengembalikan **100% persis** termasuk quirk ROM (`hispeed 1324600`).
- **Read-back verify** setiap tulis; mismatch = failure (apply tidak "hijau"
  palsu).
- **Guard path**: prefix framework (thermal/perf lock/charge/LMK/zram/game
  cpuset) ditolak di semua jalur tulis — termasuk snapshot buatan (tamper-proof).
- **Invariant kernel** divalidasi sebelum menyentuh device:
  `sched_downmigrate < sched_upmigrate` dan `gpu.min_pwrlevel > gpu.max_pwrlevel`
  → urutan tulis diturunkan otomatis dari nilai live.
- **Pass-2 otomatis**: pindah ke `schedutil` membuat node `policyN/schedutil/`
  muncul → apply melakukan re-plan sekali untuk mengisi key yang sebelumnya
  "node missing".
- **LOCKED dilaporkan** (tidak pernah setengah diterapkan): node yang
  ditolak kernel/SELinux/ROM (mis. `workqueue.power_efficient` = 444 di ROM ini,
  governor GPU selain `msm-adreno-tz` = EINVAL) muncul sebagai chip⚠ + alasan.

## Build & test

```bash
# Rust core (host tests + cross build arm64)
cd core && cargo test
ANDROID_HOME=$HOME/Android/Sdk cargo ndk -t arm64-v8a build --release
# binary: core/target/aarch64-linux-android/release/miui-ft

# Aplikasi
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk

# CLI di device (root)
adb shell su -c /data/local/tmp/mifinetune/miui-ft status
adb shell su -c /data/local/tmp/mifinetune/miui-ft apply game --json
```

## Layout device

| Path | Isi |
|---|---|
| `/data/local/tmp/mifinetune/miui-ft` | binary Rust (di-deploy app, chmod 755) |
| `/data/adb/mifinetune/profiles.json` | definisi profile (di-sync dari aset jika berbeda) |
| `/data/adb/mifinetune/snapshot.json` | nilai stock (dihapus saat restore) |
| `/data/adb/mifinetune/state.json` | profile aktif terakhir |

Migrasi ke **full Rust** direncanakan: seluruh keputusan tuning sudah berada
di crate `mifinetune-core`; Kotlin tinggal lapisan UI/bridge.

## Lisensi

Apache-2.0.
