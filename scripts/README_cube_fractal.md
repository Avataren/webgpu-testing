# Cube Fractal Scripts

This set of scripts recreates the rotating cube fractal from the original C++ engine's `CreateTestScene2()` function.

## Files

- **cube_fractal.rn**: Main fractal generation script. Creates a recursive cube structure with 5 levels of depth.
- **fractal_cube_rotate.rn**: Rotation handler for individual cube nodes in the fractal.
- **spawn_cube_fractal.rn**: Simple launcher script to spawn the fractal.

## Usage

### Option 1: Attach to an existing entity

```rune
// From another script:
let entity = spawn_entity(Some("My Fractal"));
attach_script(entity, "scripts/cube_fractal.rn");
```

### Option 2: Use the spawn script

```rune
// Attach spawn_cube_fractal.rn to any entity
attach_script(my_entity, "scripts/spawn_cube_fractal.rn");
```

## Parameters

The fractal has the following configurable constants (in cube_fractal.rn):

- `FRACTAL_ITERATIONS = 5`: Number of recursive levels (depth of the fractal)
- `FRACTAL_SPEED = 1.0`: Base rotation speed
- `ROOT_SCALE = 0.75`: Scale of the root cube
- `BRANCH_SCALE = 0.42`: Scale of child cubes relative to parent
- `BRANCH_OFFSET = 1.42`: Distance of child cubes from parent

## How it works

1. The root entity is created with a cube mesh and positioned in the scene
2. For each recursion level, the script creates 6 child cubes (one in each cardinal direction: ±X, ±Y, ±Z)
3. Each child is positioned at an offset from its parent and scaled down
4. The script avoids creating a child in the opposite direction from where it came (to prevent immediate backtracking)
5. Each cube rotates around its offset axis, creating a mesmerizing spinning fractal effect
6. The rotation speed alternates between positive and negative based on recursion depth, creating complex motion patterns

## Original C++ Implementation

This recreates the behavior of:
- `CreateTestScene2()`: Main scene creation with root entity setup
- `AddNextRecursion()`: Recursive child cube generation with parent-child relationships

## Notes

- The total number of cubes created is approximately 6^5 = 7,776 cubes (in practice fewer due to backtracking prevention)
- Each cube has its own rotation script instance
- The fractal uses parent-child transform hierarchies, so child rotations are relative to their parents
- Original material randomization is not implemented (would require material system integration)
