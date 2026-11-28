# In first person, does a rifle bob as the player walks, as a pistol does?

Priority: P2
Needs: a person at the keyboard, or timed movement keys; video of the first-person view;
any save outdoors on flat ground with the Varmint Rifle and the 9mm Pistol (the
Goodsprings start); the console for the crippled leg
Spec: #4682, [animation.md](../../spec-notes/animation.md) §9.5 the first-person rig;
[player-body.md](../../design/player-body.md) §1.4a

The first-person folder has movement clips for `mt` (unarmed), `1hp` and `1hm` only.
Standing, the `1hp` and `1hm` ones bob `Bip01 Translate` (the arms) and `Camera1st` (the
view); the `mt` ones move nothing but the root. Limping it is the other way: `hurt\mt*`
bob the arms, `hurt\1hp*` and `hurt\1hm*` don't [observed: animation.md §9.5]. Before
#4682, a rifle in first person walked on the `mt` clips, so with Weapon Inertia off it
did not bob at all, while a pistol bobbed from its own clips [observed: the motion trace,
#4682]. Ours since #4682: standing, a two-handed weapon plays the `1hp` clips (`2hm` the
`1hm` ones) before `mt` [hypothesis]; limping and sneaking are as before.

In the original, with no mods, in first person on flat ground, record each of these:
stand 2 s, walk forward 4 s, run forward 4 s, walk left 2 s, walk back 2 s.

1. The 9mm Pistol drawn.
2. The Varmint Rifle drawn.
3. Fists raised (unarmed), for the `mt` clips themselves.
4. The rifle and the pistol with a crippled leg (`player.damageav LeftMobilityCondition
   100`), walking forward only.

For each, report:

- whether the arms and weapon bob, and whether the view itself rises and falls;
- how often (one bob per footstep, or one per stride) and how much, compared with the
  pistol: the same, smaller or larger;
- whether the rifle's bob looks slower or heavier than the pistol's, beyond its size.

The lag behind a turn is not part of this check (Weapon Inertia, §1.4a).

Reading the answer: the rifle bobs as the pistol does → the stand-in (A) is right; it
bobs in another way → a bob made by code (B), to be measured from the video; it does not
bob → the original walks a rifle on `mt` (C), and #4682 goes back to the owner.
