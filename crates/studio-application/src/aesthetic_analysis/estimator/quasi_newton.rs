//! Bounded L-BFGS history with positive-curvature and ascent safeguards.

#[derive(Default)]
pub(super) struct QuasiNewton {
    previous: Option<(Vec<f64>, Vec<f64>)>,
    history: std::collections::VecDeque<(Vec<f64>, Vec<f64>, f64)>,
}
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
impl QuasiNewton {
    pub(super) fn direction(
        &mut self,
        scores: &[f64],
        gradient: &mut [f64],
        curvature: &[f64],
    ) -> f64 {
        let raw = gradient.to_vec();
        if let Some((old_scores, old_gradient)) = self.previous.take() {
            let s: Vec<_> = scores.iter().zip(old_scores).map(|(a, b)| a - b).collect();
            let y: Vec<_> = old_gradient.iter().zip(&raw).map(|(a, b)| a - b).collect();
            let sy = dot(&s, &y);
            if sy > 1e-12 * dot(&s, &s).sqrt() * dot(&y, &y).sqrt() && sy > 0.0 {
                if self.history.len() == 8 {
                    self.history.pop_front();
                }
                self.history.push_back((s, y, 1.0 / sy));
            }
        }
        self.previous = Some((scores.to_vec(), raw.clone()));
        let mut alphas = vec![0.0; self.history.len()];
        for (index, (s, y, rho)) in self.history.iter().enumerate().rev() {
            let alpha = rho * dot(s, gradient);
            alphas[index] = alpha;
            for (q, y) in gradient.iter_mut().zip(y) {
                *q -= alpha * y;
            }
        }
        for (q, c) in gradient.iter_mut().zip(curvature) {
            *q /= c;
        }
        for ((s, y, rho), alpha) in self.history.iter().zip(alphas) {
            let beta = rho * dot(y, gradient);
            for (q, s) in gradient.iter_mut().zip(s) {
                *q += s * (alpha - beta);
            }
        }
        let mut directional = dot(&raw, gradient);
        if !directional.is_finite() || directional <= 0.0 {
            self.history.clear();
            for ((q, g), c) in gradient.iter_mut().zip(&raw).zip(curvature) {
                *q = 0.9 * g / c;
            }
            directional = dot(&raw, gradient);
        }
        directional
    }
}
