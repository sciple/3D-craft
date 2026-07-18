use glam::{DVec2, DVec3};

/// An oriented plane with an orthonormal (u, v, normal) basis, used to
/// project 3D face-loop points into a 2D local coordinate system for
/// triangulation and face-detection, and back.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub origin: DVec3,
    pub normal: DVec3,
    pub u: DVec3,
    pub v: DVec3,
}

impl Plane {
    pub fn from_normal(origin: DVec3, normal: DVec3) -> Self {
        let normal = normal.normalize();
        // Any reference not parallel to `normal` works; picking the world axis
        // least aligned with `normal` keeps the basis numerically stable.
        let reference = if normal.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        // This order (v first, then u = v x normal) is what makes the basis
        // reduce to u=+X, v=+Y for the ground plane (normal=+Z) - the
        // opposite cross-product order gives a basis that's still orthonormal
        // and right-handed, but rotated 90 degrees from the natural ground
        // axes, which showed up as shapes drawing into the wrong quadrant.
        let v = normal.cross(reference).normalize();
        let u = v.cross(normal);
        Plane { origin, normal, u, v }
    }

    pub fn to_2d(&self, p: DVec3) -> DVec2 {
        let d = p - self.origin;
        DVec2::new(d.dot(self.u), d.dot(self.v))
    }

    pub fn to_3d(&self, p: DVec2) -> DVec3 {
        self.origin + self.u * p.x + self.v * p.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_plane_basis_matches_world_xy_axes() {
        // The most common plane (drawing on the ground) must map its local
        // (x, y) directly onto world (x, y, 0) - any other orientation here
        // makes shapes appear rotated/in the wrong quadrant relative to
        // where the user actually clicked.
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        assert!((plane.u - DVec3::X).length() < 1e-9);
        assert!((plane.v - DVec3::Y).length() < 1e-9);
    }

    #[test]
    fn basis_is_right_handed_for_an_arbitrary_normal() {
        let plane = Plane::from_normal(DVec3::new(1.0, 2.0, 3.0), DVec3::new(0.3, 0.7, 0.5));
        assert!((plane.u.cross(plane.v) - plane.normal).length() < 1e-9);
        assert!(plane.u.dot(plane.v).abs() < 1e-9);
    }

    #[test]
    fn to_3d_and_to_2d_round_trip() {
        let plane = Plane::from_normal(DVec3::new(0.0, 0.0, 5.0), DVec3::Z);
        let p2 = DVec2::new(3.0, -4.0);
        let p3 = plane.to_3d(p2);
        assert_eq!(p3, DVec3::new(3.0, -4.0, 5.0));
        assert!((plane.to_2d(p3) - p2).length() < 1e-9);
    }
}
