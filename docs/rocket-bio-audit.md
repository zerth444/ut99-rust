# Rocket Launcher and Bio Rifle audit

Reference: GOTY `Botpack.u` class defaults, embedded UnrealScript and vertex meshes, plus the exported `UT_Eightball.uc` and `UT_BioRifle.uc`. Original packages and local exports remain excluded from Git.

## Corrected behavior

| Path | Reference and implementation |
| --- | --- |
| Rocket loading | Start with one consumed round and `Rotate1`. Alternate the numbered rotation and loading sequences. Consume the next round after its loading animation finishes. Release interrupts either phase. Fire automatically at six rounds or when ammunition runs out. |
| Rocket firing | Read `LoadAnim`, `RotateAnim` and `FireAnim` arrays from serialized class defaults. Five and six rounds use `Fire2` and `Fire3`; selecting the similarly named Fire5/Fire6 clips would be incorrect. Firing rate multiplier is 0.6. |
| Rocket recovery | Play the appropriate firing clip, then `Load1` and `Loading` if ammunition remains. Block another shot until recovery completes. Skip empty reloads. |
| Rocket sound | Primary fire uses `RocketMk2`'s `UnrealShare.Eightball.Ignite`. The weapon's `FireSound` is unset in GOTY and previously produced silence. Alternate fire retains `EightAltFire`. Rotation uses `BarrelMove`, loading uses `Loading`. |
| Bio charge | Play `Charging` at 0.24, then hold `Loaded`. Play `BioAltRep` once at the start. Accumulate elapsed charge time when consuming each additional round, using the original strict greater-than-0.5-second threshold. Clamp released charge size to 4.1. |
| Bio fire | Both modes use the original `GelShot`. Primary selects `AltFireSound` and plays Fire at 1.05 for a player. Charged release selects `FireSound` and plays Fire at 0.4. Recovery follows the selected clip duration. |
| Bio idle movement | Play the original 18-frame `Walking` sequence at 0.3 above the original movement threshold of 10 units per second. |

The projectile renderer has a pre-existing factor of two baked into Bio meshes. Released glob scale is therefore `(1 + 0.8 * charge_size) / 2` in the current renderer, preserving the original final size.

## Verification

`deck16_rocket_and_bio_animation_and_sound_sequences` loads real GOTY assets. It checks both rocket modes at every salvo size, animation arrays and rates, rotation/loading poses, sound selection, ammunition, recovery, release during loading and immediate last-round fire. It also checks Bio charging, its maximum charge, held pose, sounds, primary cadence and walking pose. Existing projectile tests cover gel surface animations, splitting, timed detonation, grenade bounces and water entry.

All 39 tests passed with local-package tests enabled. Additional silent software-rendered captures were inspected for rocket loading/firing and Bio charging/release. No sound was played during verification; sound checks verify decoded asset availability, selection and event timing. These checks do not establish full audiovisual parity with a running original client.

## Remaining fidelity work

- Native animation tweening between sequences (including 0.05-second weapon transitions, Bio's 0.5-second loaded tween and idle/walking transitions).
- Weapon-specific view shake and instantaneous screen flash/fog; animated vertex recoil is present but does not replace these effects.
- Per-event sound volume, channel replacement and original audio spatialization. The current shared sound queue does not preserve the original low-volume barrel rotation or other native mixing parameters.
- Exact rocket spawn ring, yaw spread, grenade salvo perturbations and subsequent-rocket speed variation. The current salvo geometry remains an approximation.
- Instant-rocket player preference, native idle animation timing, complete projectile lighting/material fidelity, and frame-by-frame comparison against the original executable.

This audit fixes specific regressions and records outstanding differences. It does not mark either weapon as fully ported.
