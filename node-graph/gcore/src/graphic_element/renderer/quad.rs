use glam::{DAffine2, DVec2};

#[derive(Debug, Clone, Default, Copy)]
/// A quad defined by four vertices.
pub struct Quad(pub [DVec2; 4]);

impl Quad {
	/// Create a zero sized quad at the point
	pub fn from_point(point: DVec2) -> Self {
		Self([point; 4])
	}

	/// Convert a box defined by two corner points to a quad.
	pub fn from_box(bbox: [DVec2; 2]) -> Self {
		let size = bbox[1] - bbox[0];
		Self([bbox[0], bbox[0] + size * DVec2::X, bbox[1], bbox[0] + size * DVec2::Y])
	}

	/// Get all the edges in the quad.
	pub fn bezier_lines(&self) -> impl Iterator<Item = bezier_rs::Bezier> + '_ {
		[[self.0[0], self.0[1]], [self.0[1], self.0[2]], [self.0[2], self.0[3]], [self.0[3], self.0[0]]]
			.into_iter()
			.map(|[start, end]| bezier_rs::Bezier::from_linear_dvec2(start, end))
	}

	/// Generates a [crate::vector::Subpath] of the quad
	pub fn subpath(&self) -> crate::vector::Subpath {
		crate::vector::Subpath::from_points(self.0.into_iter(), true)
	}

	/// Generates the axis aligned bounding box of the quad
	pub fn bounding_box(&self) -> [DVec2; 2] {
		[
			self.0.into_iter().reduce(|a, b| a.min(b)).unwrap_or_default(),
			self.0.into_iter().reduce(|a, b| a.max(b)).unwrap_or_default(),
		]
	}

	/// Gets the center of a quad
	pub fn center(&self) -> DVec2 {
		self.0.iter().sum::<DVec2>() / 4.
	}

	/// Take the outside bounds of two axis aligned rectangles, which are defined by two corner points.
	pub fn combine_bounds(a: [DVec2; 2], b: [DVec2; 2]) -> [DVec2; 2] {
		[a[0].min(b[0]), a[1].max(b[1])]
	}

	/// Expand a quad by a certain amount on all sides.
	///
	/// Not currently very optimised
	pub fn inflate(&self, offset: f64) -> Quad {
		let offset = |index_before, index, index_after| {
			let [point_before, point, point_after]: [DVec2; 3] = [self.0[index_before], self.0[index], self.0[index_after]];
			let [line_in, line_out] = [point - point_before, point_after - point];
			let angle = line_in.angle_between(-line_out);
			let offset_length = offset / (std::f64::consts::FRAC_PI_2 - angle / 2.).cos();
			point + (line_in.perp().normalize_or_zero() + line_out.perp().normalize_or_zero()).normalize_or_zero() * offset_length
		};
		Self([offset(3, 0, 1), offset(0, 1, 2), offset(1, 2, 3), offset(2, 3, 0)])
	}

	/// Does this quad contain a point
	///
	/// Code from https://wrfranklin.org/Research/Short_Notes/pnpoly.html
	pub fn contains(&self, p: DVec2) -> bool {
		let mut inside = false;
		for (i, j) in (0..4).zip([3, 0, 1, 2]) {
			if (self.0[i].y > p.y) != (self.0[j].y > p.y) && p.x < (self.0[j].x - self.0[i].x * (p.y - self.0[i].y) / (self.0[j].y - self.0[i].y) + self.0[i].x) {
				inside = !inside;
			}
		}
		inside
	}
}

impl core::ops::Mul<Quad> for DAffine2 {
	type Output = Quad;

	fn mul(self, rhs: Quad) -> Self::Output {
		Quad(rhs.0.map(|point| self.transform_point2(point)))
	}
}
#[test]
fn offset_quad() {
	fn eq(a: Quad, b: Quad) -> bool {
		a.0.iter().zip(b.0).all(|(a, b)| a.abs_diff_eq(b, 0.0001))
	}

	assert!(eq(Quad::from_box([DVec2::ZERO, DVec2::ONE]).inflate(0.5), Quad::from_box([DVec2::splat(-0.5), DVec2::splat(1.5)])));
	assert!(eq(Quad::from_box([DVec2::ONE, DVec2::ZERO]).inflate(0.5), Quad::from_box([DVec2::splat(1.5), DVec2::splat(-0.5)])));
	assert!(eq(
		(DAffine2::from_scale(DVec2::new(-1., 1.)) * Quad::from_box([DVec2::ZERO, DVec2::ONE])).inflate(0.5),
		DAffine2::from_scale(DVec2::new(-1., 1.)) * Quad::from_box([DVec2::splat(-0.5), DVec2::splat(1.5)])
	));
}
#[test]
fn quad_contains() {
	assert!(Quad::from_box([DVec2::ZERO, DVec2::ONE]).contains(DVec2::splat(0.5)));
	assert!(Quad::from_box([DVec2::ONE, DVec2::ZERO]).contains(DVec2::splat(0.5)));
	assert!((DAffine2::from_scale(DVec2::new(-1., 1.)) * Quad::from_box([DVec2::ZERO, DVec2::ONE])).contains(DVec2::new(-0.5, 0.5)));

	assert!(!Quad::from_box([DVec2::ZERO, DVec2::ONE]).contains(DVec2::new(1., 1.1)));
	assert!(!Quad::from_box([DVec2::ONE, DVec2::ZERO]).contains(DVec2::new(0.5, -0.01)));
	assert!(!(DAffine2::from_scale(DVec2::new(-1., 1.)) * Quad::from_box([DVec2::ZERO, DVec2::ONE])).contains(DVec2::splat(0.5)));
}
