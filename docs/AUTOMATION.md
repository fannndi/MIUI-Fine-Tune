# Automation API

Tasker, MacroDroid, or plain `adb` can drive MiFineTune through a small,
deliberately limited broadcast surface. The receiver ([`AutomationReceiver`]
in the app) only routes the same actions the UI already has — it never
starts or stops the Service, and all decisions stay in the Rust daemon.

## Actions

| Action | Extras | Effect |
|---|---|---|
| `com.mifinetune.action.SET_PROFILE` | `profile` (string): `powersave` \| `balance` \| `game` | Writes the base profile and applies it now when the Service is on; otherwise it takes effect at the next Service start |
| `com.mifinetune.action.BOOST` | – | Raises one manual 5 s boost window (the hidden `boost` profile), then the daemon returns to the normal decision. No cooldown (unlike jank auto-boost) |
| `com.mifinetune.action.SET_CHARGE_LIMIT` | `enabled` (boolean), `pct` (int, 60–95, optional) | Toggles the charge limit / sets its percentage. The daemon reacts on its next config sync |

Requires the MiFineTune Service to be **on** for immediate effects
(background tuning is daemon-owned).

## adb examples

```bash
PKG=com.mifinetune/.dynamic.AutomationReceiver
adb shell am broadcast -n $PKG -a com.mifinetune.action.SET_PROFILE --es profile game
adb shell am broadcast -n $PKG -a com.mifinetune.action.BOOST
adb shell am broadcast -n $PKG -a com.mifinetune.action.SET_CHARGE_LIMIT --ez enabled true --ei pct 80
adb shell am broadcast -n $PKG -a com.mifinetune.action.SET_CHARGE_LIMIT --ez enabled false
```

The explicit component (`-n`) is recommended on modern Android; the actions
are also declared as an intent filter, so an implicit broadcast works too.

## Tasker

1. Action: **Net → Send Intent** (or *System → Send Intent*).
2. Action: `com.mifinetune.action.SET_PROFILE`
3. Package: `com.mifinetune`
4. Extra: `profile:game` (Tasker extra syntax: name `profile`, value `game`)
5. Target: **Broadcast Receiver**

Same pattern for `BOOST` (no extras) and `SET_CHARGE_LIMIT`
(`enabled:true` / `pct:80`).

## Safety notes

- The receiver is `exported` (automation tools need that); the surface is
  intentionally read/write-scoped to Device-Profile selection, a transient
  boost window, and the charge limit. Service on/off is not reachable.
- Config writes are atomic and app-owned; the daemon reloads on the
  `config_changed` hint.
- `SET_PROFILE` with an unknown id is ignored (logged under tag
  `MiFineTune`).
