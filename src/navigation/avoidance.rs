//! Optimal reciprocal collision avoidance (ORCA): each agent picks the velocity
//! nearest the one it wants that keeps it from touching its neighbours within a time
//! horizon, assuming they do their share. A port of the RVO2 library's solver
//! (van den Berg et al.), in the ground plane (x, z as x, y).

use glam::Vec2;

const EPSILON: f32 = 0.00001;

/// A half-plane of allowed velocities: those left of the line through `point` along
/// `direction`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub point: Vec2,
    pub direction: Vec2,
}

/// Another agent (or a moving obstacle) as the solver sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbour {
    pub position: Vec2,
    pub velocity: Vec2,
    pub radius: f32,
    /// The share of avoiding this neighbour that falls to us: 0.5 between equals, 1
    /// for something that will not move aside.
    pub responsibility: f32,
}

/// What one agent knows when it chooses its velocity.
pub struct AvoidanceInput<'a> {
    pub position: Vec2,
    pub velocity: Vec2,
    pub preferred_velocity: Vec2,
    pub radius: f32,
    pub max_speed: f32,
    pub time_horizon: f32,
    pub time_horizon_obstacles: f32,
    /// The length of the simulation step (for agents already overlapping).
    pub time_step: f32,
    pub neighbours: &'a [Neighbour],
    /// Wall segments (navmesh edges with nothing beyond them), walkable side on the right
    /// going from the first point to the second (it decides for an agent on the line).
    pub walls: &'a [(Vec2, Vec2)],
    /// How far the agent keeps from walls: the navmesh already stops an agent radius
    /// short of what it bounds, so this is a sliver, not the agent's radius.
    pub wall_radius: f32,
}

/// Within this of a wall's line (m), an agent counts as standing on it.
const ON_WALL: f32 = 0.01;

fn det(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// One end of a wall as RVO2's obstacles see it: the next vertex is the segment's
/// other end, walked so the agent stands on its right.
#[derive(Clone, Copy)]
struct ObstacleVertex {
    point: Vec2,
    unit_dir: Vec2,
}

/// The velocity an agent should take this step.
pub fn compute_velocity(input: &AvoidanceInput) -> Vec2 {
    let mut lines: Vec<Line> = Vec::with_capacity(input.walls.len() + input.neighbours.len());
    let position = input.position;
    let velocity = input.velocity;
    let radius = input.radius;
    let inv_time_horizon_obst = 1.0 / input.time_horizon_obstacles.max(EPSILON);

    for &(a, b) in input.walls {
        if (b - a).length_squared() < EPSILON {
            continue;
        }
        let radius = input.wall_radius.max(0.01);
        // Walk the segment so the agent is on its right (RVO2 only sees that side). On
        // the line (pushed to the navmesh's edge) it is on the walkable side: otherwise
        // the wall would only let it move off the navmesh.
        let left_of = det(a - position, b - a);
        let on_line = left_of.abs() <= ON_WALL * (b - a).length();
        let (p1, p2) = if left_of < 0.0 || on_line {
            (a, b)
        } else {
            (b, a)
        };
        let obstacle1 = ObstacleVertex {
            point: p1,
            unit_dir: (p2 - p1).normalize(),
        };
        let obstacle2 = ObstacleVertex {
            point: p2,
            unit_dir: (p1 - p2).normalize(),
        };

        let relative_position1 = obstacle1.point - position;
        let relative_position2 = obstacle2.point - position;

        // Already covered by an earlier wall's line?
        let already_covered = lines.iter().any(|line| {
            det(
                inv_time_horizon_obst * relative_position1 - line.point,
                line.direction,
            ) - inv_time_horizon_obst * radius
                >= -EPSILON
                && det(
                    inv_time_horizon_obst * relative_position2 - line.point,
                    line.direction,
                ) - inv_time_horizon_obst * radius
                    >= -EPSILON
        });
        if already_covered {
            continue;
        }

        let dist_sq1 = relative_position1.length_squared();
        let dist_sq2 = relative_position2.length_squared();
        let radius_sq = radius * radius;
        let obstacle_vector = obstacle2.point - obstacle1.point;
        let s = (-relative_position1).dot(obstacle_vector) / obstacle_vector.length_squared();
        let dist_sq_line = (-relative_position1 - s * obstacle_vector).length_squared();

        if s < 0.0 && dist_sq1 <= radius_sq {
            // Touching the left end.
            lines.push(Line {
                point: Vec2::ZERO,
                direction: Vec2::new(-relative_position1.y, relative_position1.x)
                    .normalize_or_zero(),
            });
            continue;
        } else if s > 1.0 && dist_sq2 <= radius_sq {
            // Touching the right end (when the next edge does not take care of it).
            if det(relative_position2, obstacle2.unit_dir) >= 0.0 {
                lines.push(Line {
                    point: Vec2::ZERO,
                    direction: Vec2::new(-relative_position2.y, relative_position2.x)
                        .normalize_or_zero(),
                });
            }
            continue;
        } else if (0.0..1.0).contains(&s) && dist_sq_line <= radius_sq {
            // Touching the segment.
            lines.push(Line {
                point: Vec2::ZERO,
                direction: -obstacle1.unit_dir,
            });
            continue;
        }

        // No collision: the legs of the velocity obstacle.
        let (o1, o2, left_leg_direction, right_leg_direction);
        if s < 0.0 && dist_sq_line <= radius_sq {
            // Seen obliquely: the left end alone shapes it.
            o1 = obstacle1;
            o2 = obstacle1;
            let leg1 = (dist_sq1 - radius_sq).sqrt();
            left_leg_direction = Vec2::new(
                relative_position1.x * leg1 - relative_position1.y * radius,
                relative_position1.x * radius + relative_position1.y * leg1,
            ) / dist_sq1;
            right_leg_direction = Vec2::new(
                relative_position1.x * leg1 + relative_position1.y * radius,
                -relative_position1.x * radius + relative_position1.y * leg1,
            ) / dist_sq1;
        } else if s > 1.0 && dist_sq_line <= radius_sq {
            // The right end alone.
            o1 = obstacle2;
            o2 = obstacle2;
            let leg2 = (dist_sq2 - radius_sq).sqrt();
            left_leg_direction = Vec2::new(
                relative_position2.x * leg2 - relative_position2.y * radius,
                relative_position2.x * radius + relative_position2.y * leg2,
            ) / dist_sq2;
            right_leg_direction = Vec2::new(
                relative_position2.x * leg2 + relative_position2.y * radius,
                -relative_position2.x * radius + relative_position2.y * leg2,
            ) / dist_sq2;
        } else {
            o1 = obstacle1;
            o2 = obstacle2;
            let leg1 = (dist_sq1 - radius_sq).max(0.0).sqrt();
            left_leg_direction = Vec2::new(
                relative_position1.x * leg1 - relative_position1.y * radius,
                relative_position1.x * radius + relative_position1.y * leg1,
            ) / dist_sq1;
            let leg2 = (dist_sq2 - radius_sq).max(0.0).sqrt();
            right_leg_direction = Vec2::new(
                relative_position2.x * leg2 + relative_position2.y * radius,
                -relative_position2.x * radius + relative_position2.y * leg2,
            ) / dist_sq2;
        }

        // Legs never point into the neighbouring edge (on a two-vertex obstacle, the
        // vertex before either end is the other end).
        let left_neighbour = if (o1.point - obstacle1.point).length_squared() < EPSILON {
            obstacle2
        } else {
            obstacle1
        };
        let mut left_leg_direction = left_leg_direction;
        let mut right_leg_direction = right_leg_direction;
        let mut left_leg_foreign = false;
        let mut right_leg_foreign = false;
        if det(left_leg_direction, -left_neighbour.unit_dir) >= 0.0 {
            left_leg_direction = -left_neighbour.unit_dir;
            left_leg_foreign = true;
        }
        if det(right_leg_direction, o2.unit_dir) <= 0.0 {
            right_leg_direction = o2.unit_dir;
            right_leg_foreign = true;
        }

        let left_cutoff = inv_time_horizon_obst * (o1.point - position);
        let right_cutoff = inv_time_horizon_obst * (o2.point - position);
        let cutoff_vec = right_cutoff - left_cutoff;
        let same = (o1.point - o2.point).length_squared() < EPSILON;
        let t = if same {
            0.5
        } else {
            (velocity - left_cutoff).dot(cutoff_vec) / cutoff_vec.length_squared()
        };
        let t_left = (velocity - left_cutoff).dot(left_leg_direction);
        let t_right = (velocity - right_cutoff).dot(right_leg_direction);

        if (t < 0.0 && t_left < 0.0) || (same && t_left < 0.0 && t_right < 0.0) {
            // Project on the left cut-off circle.
            let unit_w = (velocity - left_cutoff).normalize_or_zero();
            lines.push(Line {
                direction: Vec2::new(unit_w.y, -unit_w.x),
                point: left_cutoff + radius * inv_time_horizon_obst * unit_w,
            });
            continue;
        } else if t > 1.0 && t_right < 0.0 {
            // Project on the right cut-off circle.
            let unit_w = (velocity - right_cutoff).normalize_or_zero();
            lines.push(Line {
                direction: Vec2::new(unit_w.y, -unit_w.x),
                point: right_cutoff + radius * inv_time_horizon_obst * unit_w,
            });
            continue;
        }

        let dist_sq_cutoff = if t < 0.0 || t > 1.0 || same {
            f32::INFINITY
        } else {
            (velocity - (left_cutoff + t * cutoff_vec)).length_squared()
        };
        let dist_sq_left = if t_left < 0.0 {
            f32::INFINITY
        } else {
            (velocity - (left_cutoff + t_left * left_leg_direction)).length_squared()
        };
        let dist_sq_right = if t_right < 0.0 {
            f32::INFINITY
        } else {
            (velocity - (right_cutoff + t_right * right_leg_direction)).length_squared()
        };

        if dist_sq_cutoff <= dist_sq_left && dist_sq_cutoff <= dist_sq_right {
            let direction = -o1.unit_dir;
            lines.push(Line {
                direction,
                point: left_cutoff
                    + radius * inv_time_horizon_obst * Vec2::new(-direction.y, direction.x),
            });
        } else if dist_sq_left <= dist_sq_right {
            if left_leg_foreign {
                continue;
            }
            let direction = left_leg_direction;
            lines.push(Line {
                direction,
                point: left_cutoff
                    + radius * inv_time_horizon_obst * Vec2::new(-direction.y, direction.x),
            });
        } else {
            if right_leg_foreign {
                continue;
            }
            let direction = -right_leg_direction;
            lines.push(Line {
                direction,
                point: right_cutoff
                    + radius * inv_time_horizon_obst * Vec2::new(-direction.y, direction.x),
            });
        }
    }

    let obstacle_lines = lines.len();
    let inv_time_horizon = 1.0 / input.time_horizon.max(EPSILON);

    for other in input.neighbours {
        let relative_position = other.position - position;
        let relative_velocity = velocity - other.velocity;
        let dist_sq = relative_position.length_squared();
        let combined_radius = radius + other.radius;
        let combined_radius_sq = combined_radius * combined_radius;
        let (direction, u);
        if dist_sq > combined_radius_sq {
            let w = relative_velocity - inv_time_horizon * relative_position;
            let w_length_sq = w.length_squared();
            let dot1 = w.dot(relative_position);
            if dot1 < 0.0 && dot1 * dot1 > combined_radius_sq * w_length_sq {
                // Project on the cut-off circle.
                let w_length = w_length_sq.sqrt();
                let unit_w = w / w_length;
                direction = Vec2::new(unit_w.y, -unit_w.x);
                u = (combined_radius * inv_time_horizon - w_length) * unit_w;
            } else {
                // Project on the legs.
                let leg = (dist_sq - combined_radius_sq).sqrt();
                direction = if det(relative_position, w) > 0.0 {
                    Vec2::new(
                        relative_position.x * leg - relative_position.y * combined_radius,
                        relative_position.x * combined_radius + relative_position.y * leg,
                    ) / dist_sq
                } else {
                    -Vec2::new(
                        relative_position.x * leg + relative_position.y * combined_radius,
                        -relative_position.x * combined_radius + relative_position.y * leg,
                    ) / dist_sq
                };
                let dot2 = relative_velocity.dot(direction);
                u = dot2 * direction - relative_velocity;
            }
        } else {
            // Already overlapping: get apart within this step.
            let inv_time_step = 1.0 / input.time_step.max(EPSILON);
            let w = relative_velocity - inv_time_step * relative_position;
            let w_length = w.length();
            let unit_w = if w_length > EPSILON {
                w / w_length
            } else {
                Vec2::X
            };
            direction = Vec2::new(unit_w.y, -unit_w.x);
            u = (combined_radius * inv_time_step - w_length) * unit_w;
        }
        lines.push(Line {
            point: velocity + other.responsibility * u,
            direction,
        });
    }

    let mut result = Vec2::ZERO;
    let fail = linear_program2(
        &lines,
        input.max_speed,
        input.preferred_velocity,
        false,
        &mut result,
    );
    if fail < lines.len() {
        linear_program3(&lines, obstacle_lines, fail, input.max_speed, &mut result);
    }
    result
}

fn linear_program1(
    lines: &[Line],
    line_no: usize,
    radius: f32,
    opt_velocity: Vec2,
    direction_opt: bool,
    result: &mut Vec2,
) -> bool {
    let line = lines[line_no];
    let dot = line.point.dot(line.direction);
    let discriminant = dot * dot + radius * radius - line.point.length_squared();
    if discriminant < 0.0 {
        return false;
    }
    let sqrt_discriminant = discriminant.sqrt();
    let mut t_left = -dot - sqrt_discriminant;
    let mut t_right = -dot + sqrt_discriminant;
    for other in &lines[..line_no] {
        let denominator = det(line.direction, other.direction);
        let numerator = det(other.direction, line.point - other.point);
        if denominator.abs() <= EPSILON {
            if numerator < 0.0 {
                return false;
            }
            continue;
        }
        let t = numerator / denominator;
        if denominator >= 0.0 {
            t_right = t_right.min(t);
        } else {
            t_left = t_left.max(t);
        }
        if t_left > t_right {
            return false;
        }
    }
    if direction_opt {
        *result = if opt_velocity.dot(line.direction) > 0.0 {
            line.point + t_right * line.direction
        } else {
            line.point + t_left * line.direction
        };
    } else {
        let t = line.direction.dot(opt_velocity - line.point);
        *result = line.point + t.clamp(t_left, t_right) * line.direction;
    }
    true
}

fn linear_program2(
    lines: &[Line],
    radius: f32,
    opt_velocity: Vec2,
    direction_opt: bool,
    result: &mut Vec2,
) -> usize {
    *result = if direction_opt {
        opt_velocity * radius
    } else if opt_velocity.length_squared() > radius * radius {
        opt_velocity.normalize_or_zero() * radius
    } else {
        opt_velocity
    };
    for i in 0..lines.len() {
        if det(lines[i].direction, lines[i].point - *result) > 0.0 {
            let previous = *result;
            if !linear_program1(lines, i, radius, opt_velocity, direction_opt, result) {
                *result = previous;
                return i;
            }
        }
    }
    lines.len()
}

fn linear_program3(
    lines: &[Line],
    obstacle_lines: usize,
    begin: usize,
    radius: f32,
    result: &mut Vec2,
) {
    let mut distance = 0.0;
    for i in begin..lines.len() {
        if det(lines[i].direction, lines[i].point - *result) > distance {
            let mut projected: Vec<Line> = lines[..obstacle_lines].to_vec();
            for j in obstacle_lines..i {
                let determinant = det(lines[i].direction, lines[j].direction);
                let point = if determinant.abs() <= EPSILON {
                    if lines[i].direction.dot(lines[j].direction) > 0.0 {
                        continue;
                    }
                    0.5 * (lines[i].point + lines[j].point)
                } else {
                    lines[i].point
                        + (det(lines[j].direction, lines[i].point - lines[j].point) / determinant)
                            * lines[i].direction
                };
                projected.push(Line {
                    point,
                    direction: (lines[j].direction - lines[i].direction).normalize_or_zero(),
                });
            }
            let previous = *result;
            let opt = Vec2::new(-lines[i].direction.y, lines[i].direction.x);
            if linear_program2(&projected, radius, opt, true, result) < projected.len() {
                *result = previous;
            }
            distance = det(lines[i].direction, lines[i].point - *result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        position: Vec2,
        preferred: Vec2,
        neighbours: &'a [Neighbour],
        walls: &'a [(Vec2, Vec2)],
    ) -> AvoidanceInput<'a> {
        AvoidanceInput {
            position,
            velocity: preferred,
            preferred_velocity: preferred,
            radius: 0.4,
            max_speed: 1.5,
            time_horizon: 2.0,
            time_horizon_obstacles: 1.0,
            time_step: 1.0 / 60.0,
            neighbours,
            walls,
            wall_radius: 0.05,
        }
    }

    #[test]
    fn free_agents_keep_their_velocity() {
        let v = compute_velocity(&input(Vec2::ZERO, Vec2::new(1.0, 0.0), &[], &[]));
        assert!((v - Vec2::new(1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn head_on_agents_swerve() {
        let other = Neighbour {
            position: Vec2::new(2.0, 0.0),
            velocity: Vec2::new(-1.0, 0.0),
            radius: 0.4,
            responsibility: 0.5,
        };
        let v = compute_velocity(&input(Vec2::ZERO, Vec2::new(1.0, 0.0), &[other], &[]));
        assert!(v.y.abs() > 0.05, "swerves sideways: {v:?}");
        assert!(v.length() <= 1.5 + 1e-4);
    }

    #[test]
    fn walls_stop_agents_running_into_them() {
        // A wall across the way, 0.6 m ahead.
        let walls = [(Vec2::new(1.0, -5.0), Vec2::new(1.0, 5.0))];
        let v = compute_velocity(&input(Vec2::ZERO, Vec2::new(1.5, 0.0), &[], &walls));
        assert!(v.x < 1.5 * 0.9, "slows before the wall: {v:?}");
    }

    #[test]
    fn an_agent_on_a_wall_walks_away_from_it() {
        // Pushed onto the navmesh's edge (x = 0, walkable toward -x: on the right going
        // down -y), it can leave the wall but not cross it.
        let walls = [(Vec2::new(0.0, 5.0), Vec2::new(0.0, -5.0))];
        let away = compute_velocity(&input(Vec2::ZERO, Vec2::new(-1.5, 0.5), &[], &walls));
        assert!(away.x < -1.0, "walks away: {away:?}");
        let into = compute_velocity(&input(Vec2::ZERO, Vec2::new(1.5, 0.5), &[], &walls));
        assert!(into.x <= 1e-3, "stays out of the wall: {into:?}");
    }
}
