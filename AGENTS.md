# AGENTS.md — MiFineTune

Panduan pengembangan untuk POCO X3 NFC **surya / sm6150 / MIUI 12 / Android 10**
(root APatch). Baca juga `docs/ROM-HARMONY.md` (peta kepemilikan node) dan
`README.md` (cara pakai).

Arsitektur: **engine Rust** (`core/`, binary `miui-ft`) + **app Kotlin/Compose**
sebagai UI/orchestrator. App offline (tanpa INTERNET), semua eksekusi lewat `su`.

---

## Peta repo

| Path | Isi |
|---|---|
| `core/src/catalog.rs` | registry **84 node**: path, tier (Free/Baseline), kind, range, `scheduler_dependent`, `FORBIDDEN_*` |
| `core/src/profile.rs` | parse `profiles.json`, validator + plan, invariant pasangan, `readback_matches` |
| `core/src/apply.rs` | apply / restore / verify, snapshot, urutan tulis (`ordered_pairs_with`), pass-2 |
| `core/src/probe.rs` | pembacaan node + Options (governors, OPP, cluster cpus, core_ctl max) |
| `core/src/main.rs` | CLI: `plan/apply/verify/restore/status/probe/catalog` (JSON) |
| `core/profiles.json` | definisi profile — **di-embed via `include_str!`**, jadi ikut build |
| `app/.../core/` | `RootBridge` (su + deploy binary), `FtClient` (exec + parse JSON), `Models` |
| `app/.../ui/` | `HomeScreen` (Compose), `HomeViewModel` (state) |
| `tools/owner-map-audit.sh` | audit tier katalog vs ROM (boot + runtime writers) |
| `tools/perf-hal-runtime-writers.txt` | daftar node yang ditulis runtime oleh perf HAL/netd (dengan provenance) |
| `tools/display-off-diff.sh` | uji empiris perilaku custom saat layar mati |
| `docs/ROM-HARMONY.md` | tabel pemilik node + invariant kernel + temuan audit |

## Hard rules (jangan dilanggar)

1. **Owner map**: engine hanya menulis node tier **Free/Baseline**. Node
   **Forbidden** tidak pernah ditulis (`guard_path` di `catalog.rs`). Menambah
   entri katalog wajib disertai bukti tier (post_boot + perf XML + runtime
   writers + device).
2. **Jangan lawan framework**: kalau sebuah node ditulis runtime oleh MIUI
   (lihat `tools/perf-hal-runtime-writers.txt`) → tier minimal **Baseline**
   (coexist; drift guard memulihkan). Kalau framework menimpanya terus-menerus
   (contoh: `net.tcp_rmem/wmem` direset network stack tiap siklus display-off)
   → **jangan dipakai di profile sama sekali** (ada test otomatis untuk ini).
3. **Validator berbasis source**: setiap invariant baru harus menyertakan
   kutipan kernel (`file:line`), lihat tabel di `docs/ROM-HARMONY.md`.
4. **Asset sync (bug klasik v0.1 & v0.3)**: `app/src/main/assets/{miui-ft,profiles.json}`
   **menimpa** binary + profile pack di device saat launch (banding ukuran/isi).
   Sekarang **otomatis** lewat Gradle task `syncCore` (preBuild menyalin dari
   `core/`), tapi kalau mengubah alur build: pastikan `assembleDebug` tetap
   menyalin keduanya. Kejadian nyata: binary lama → katalog 67→59; profiles
   lama → `apply sleep` gagal "unknown profile" saat layar mati.
5. **Snapshot semantics**: apply pertama menyimpan snapshot state live.
   `restore` hanya "finalisasi" (hapus snapshot + active=None) bila
   `failed == 0`; kalau ada yang gagal, snapshot **sengaja dipertahankan**
   untuk retry (kasus freq-cap thermal sudah tertangani oleh kind `FreqMax`;
   sisa kegagalan nyata = write rejected/mismatch betulan).
6. **Pasangan & urutan kernel**: `downmigrate < upmigrate` (reorder otomatis),
   `gpu.max_pwrlevel ≤ min_pwrlevel`, `task_thres ≥ num_cpus cluster`,
   `min_cpus` di-pre-clamp ke `max_cpus`. Jangan hapus `ordered_pairs_with`
   atau pass-2 schedutil (dir `policyN/schedutil/` hilang saat governor
   `powersave` aktif, muncul lagi setelah switch).
7. **Restore = stock persis**: nilai yang tak OPP (quirk `hispeed 1324600`)
   harus kembali apa adanya; jangan "merapikan" snapshot.
8. **Uji di device itu wajib** untuk perubahan engine; unit test saja tidak
   cukup (perilaku kernel/thermal tidak bisa dimock).

## Build & test

```bash
# host
cd core && cargo test                       # 33 unit test

# cross-build arm64 + sync ke asset app (JANGAN LUPA)
ANDROID_HOME=$HOME/Android/Sdk cargo ndk -t arm64-v8a build --release
cp target/aarch64-linux-android/release/miui-ft ../app/src/main/assets/miui-ft

# push ke device (untuk test CLI)
export PATH=$PATH:$HOME/Android/Sdk/platform-tools
adb push target/aarch64-linux-android/release/miui-ft /data/local/tmp/mifinetune/miui-ft
adb shell "su -c 'chmod 755 /data/local/tmp/mifinetune/miui-ft'"

# app
cd .. && ./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Catatan build app: Gradle 9.7.1 / AGP 9.4.1 / Kotlin 2.4.20 (AGP9 built-in
Kotlin, **tanpa** plugin `kotlin.android`), Compose BOM 2026.09.00, `compileSdk 37`,
JDK 17 lokal (tanpa gradle-daemon-jvm.properties), heap 4G di `gradle.properties`.

## E2E device (pola yang dipakai terus)

```bash
B=/data/local/tmp/mifinetune/miui-ft

adb shell "su -c '$B plan powersave'"                        # cek rencana + locked
adb shell "su -c '$B apply powersave'" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['ok'], d['wrote'], d['verified'], d['failed'])"
adb shell "su -c '$B verify powersave'"                     # deteksi drift
adb shell "su -c '$B restore'"                              # kembali stock
adb shell "su -c '$B status'" | python3 -m json.tool | head # active/snapshot/catalog
```

State di device: `/data/adb/mifinetune/{profiles.json,state.json,snapshot.json}`.

## CLI `miui-ft`

| Command | Fungsi |
|---|---|
| `plan <id>` | rencana tulis + status per node (ok / unchanged / locked+alasan) — **read-only** |
| `apply <id>` | snapshot (bila perlu) → tulis → read-back verify → set active |
| `verify <id>` | banding live vs profile (read-only; deteksi drift) |
| `restore` | tulis balik snapshot; sukses = snapshot dikonsumsi |
| `status` | active, snapshot, jumlah katalog, ROM device |
| `probe` | baca semua node katalog (JSON) |
| `catalog` | dump katalog JSON (dipakai `tools/owner-map-audit.sh`) |

## Quirk perangkat (sering bikin bingung saat debug)

- Governor `powersave` aktif → **dir `policyN/schedutil/` hilang**; probe
  melaporkan `exists:false`. Ini normal; pass-2 apply mengurusnya.
- `mCurrentFocus`/`dumpsys` ok, tapi `uiautomator dump` kadang gagal pertama
  kali (`FileNotFoundException`) → retry sekali.
- stderr MIUI (`theme_compatibility.xml`) = noise, bukan crash.
- `scaling_max_freq` memakai kind **FreqMax**: saat thermal memegang cap lebih
  rendah (gold `1209600` padahal minta `1555200`), itu **in-sync** — thermal
  menang. Jangan ubah jadi exact-match (dulu itu memicu failure palsu pada
  apply/restore). Cap hilang (live > want) baru dianggap drift.
- `scaling_min_freq` memakai kind **FreqMin** (v0.4): QoS `msm_performance`
  menahan floor lebih tinggi = **in-sync** (framework menang); live < want
  baru drift. Eksperimen live: tulis 576000 saat QoS 1248000 → baca 1248000;
  QoS lepas → node kembali sendiri ke nilai yang ditulis.
- `io.scheduler` read-back: hanya token **bracketed** yang dianggap aktif —
  bug v0.3 terverifikasi: token offered unbracketed salah-positif sehingga
  apply `deadline` pernah di-skip selamanya.
- MIUI Game Turbo/perf HAL menulis `core_ctl`, cpusets, stune, GPU pwrlevel,
  QoS `msm_performance` **transient** → wajar terdeteksi drift sesaat.
- `su` dari app harus **path absolut** (`RootBridge.suBin`) — app tidak
  mewarisi PATH shell. Sama untuk binary engine: `probe.rs` memanggil
  `/system/bin/getprop` absolut.
- `dumpsys`/binder TIDAK bisa dipakai dari konteks `su` app (APatch): output
  kosong/error — peek foreground memakai event-buffer `logcat` (kernel) saja,
  dan command su jangan memakai pipe/quote (tidak selamat melewati su).
- `net.tcp_rmem/wmem` direset network stack saat siklus display-off — profile
  tidak menyentuhnya (lihat Hard rule 2).

## Automasi (v0.3 final)

**Model perilaku** (dikonfirmasi user):
- Tap kartu = apply sekarang + jadi **BASE universal** (dipakai semua app yang
  tidak dipetakan). App terpetakan menimpa base saat di depan; keluar → base.
- **Service OFF = restore stock + stop** (tidak intervensi apa pun).
- Layar mati → profile `sleep` (selalu, ±10 dtk). Wake/unlock → base/mapped.
- Fresh install: base = Balance; `enabled` default true.
- Supervisor tick 3 dtk juga **mereonsiliasi screen state** langsung dari
  `PowerManager.isInteractive` — MIUI kadang menjatuhkan broadcast `SCREEN_ON`,
  jangan andalkan broadcast saja.

**Komponen**:
- **`Tuner`** (core/): satu mutex untuk apply/verify/restore dari UI, service,
  dan drift guard (guard 15 dtk = satu call `apply` — engine yang melewati key
  unchanged, repair diputuskan di core Rust).
- **`AutomationService`** (FGS senyap, IMPORTANCE_MIN):
  - **Watcher event-driven**: root `logcat -b events -s am_resume_activity:V`
    → switch instan. Supervisor restart (backoff 10 dtk) + fallback
    `peekEvents()` (logcat -d, event terakhir) saat stream mati.
  - **PENTING: jangan pakai `dumpsys` dari su app** — di APatch, binder tidak
    bisa diakses dari konteks su app (output kosong/27 char). Gunakan `logcat`
    (kernel buffer) untuk peek; jangan tambahkan pipe/quotes di command su
    (tidak selamat).
  - **Coalescing**: burst event (task restore/keyguard) → satu apply setelah
    settle 700 ms; apply berjalan tidak menumpuk (`pendingDecision` supersede).
  - **Retry sekali** untuk apply & restore yang gagal transient (race QoS
    Game Turbo/PowerKeeper, thermal) + log key yang gagal.
  - Seed wake/unlock: `peekEvents()` + `lastRealPkg` (paket non-transient
    terakhir) → selalu evaluate sekali setelah unlock.
- **`ModeArbiter`** (murni, test JVM): layar mati → sleep; app terpetakan →
  profile-nya; lainnya → base; SystemUI/IME/app sendiri/dialog izin =
  transient (jangan switch); launcher = sinyal balik ke base; keyguard = no-op.
- **UsageStats TIDAK dipakai** (MIUI jarang mengirim event resume; permission
  PACKAGE_USAGE_STATS sudah dihapus dari manifest).
- **Screens** (English only): Home (3 baris profile + Detail, Apps Profile,
  Service switch) · Apps Profile (search + bottom sheet) · Settings (izin +
  diagnostik). Hapus/restore = switch Service (confirm bila ada snapshot).
- **Gradle `syncCore`**: menyalin `core/profiles.json` + binary rilis ke
  `app/src/main/assets/` pada setiap build (mencegah bug asset basi).

## Konvensi kode

- Rust: satu file satu tanggung jawab; komentar invariant menyertakan kutipan
  source kernel; test per aturan (lihat `catalog.rs` / `profile.rs`).
- Kotlin: komentar `Responsibility / Non-goals` di file inti; UI tidak memuat
  logika tuning; semua operasi engine melalui `FtClient`.
- Commit kecil per fase; pesan commit menjelaskan *mengapa*.
