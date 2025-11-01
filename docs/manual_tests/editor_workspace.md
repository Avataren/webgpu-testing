# Editor workspace regression checklist

Use this checklist when validating workspace-aware editor changes.

- [ ] Launch the editor and confirm the active scene appears in the hierarchy after load.
- [ ] Open or switch to a different scene tab and ensure the hierarchy, inspector selection, and environment controls reflect the new scene immediately.
- [ ] Toggle play mode on the current scene, verify animation/simulation starts, then switch tabs and confirm only the newly active scene responds to play/stop toggles.
- [ ] Modify an environment setting in any scene, switch away, and back again to verify the edited scene retains the change and is marked dirty for saving.
- [ ] Delete and reselect entities in multiple scenes to confirm selection state is preserved independently per scene.
