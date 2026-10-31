//! Average-linkage agglomerative clustering of unit vectors on cosine distance (nearest-neighbour
//! chain algorithm: O(n²) time and memory), and cutting the tree by distance or cluster count.

/// One merge: clusters `a` and `b` (indices of their first member) at distance `d`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Merge {
    pub a: usize,
    pub b: usize,
    pub d: f32,
}

/// Most vectors clustered directly (the distance matrix is `n²` floats); callers subsample.
pub const MAX_POINTS: usize = 2500;

pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

/// Scale to unit length (zero vectors stay zero).
pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-12 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// All merges of the average-linkage tree over `points` (at most [`MAX_POINTS`]; more are
/// ignored).
pub fn average_linkage(points: &[Vec<f32>]) -> Vec<Merge> {
    let n = points.len().min(MAX_POINTS);
    if n < 2 {
        return Vec::new();
    }
    let mut d = vec![0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let v = cosine_distance(&points[i], &points[j]);
            d[i * n + j] = v;
            d[j * n + i] = v;
        }
    }
    let mut size = vec![1usize; n];
    let mut active = vec![true; n];
    let mut left = n;
    let mut chain: Vec<usize> = Vec::with_capacity(n);
    let mut merges = Vec::with_capacity(n - 1);
    while left > 1 {
        if chain.is_empty() {
            let Some(first) = active.iter().position(|&a| a) else { break };
            chain.push(first);
        }
        let (a, b, dist) = loop {
            let Some(&a) = chain.last() else { break (0, 0, f32::NAN) };
            let prev = chain.len().checked_sub(2).and_then(|i| chain.get(i)).copied();
            // nearest active neighbour of a, preferring the previous chain element on ties
            let mut best = (f32::INFINITY, usize::MAX);
            if let Some(p) = prev {
                best = (d[a * n + p], p);
            }
            for k in 0..n {
                if k != a && active[k] && d[a * n + k] < best.0 {
                    best = (d[a * n + k], k);
                }
            }
            if best.1 == usize::MAX {
                break (a, a, f32::NAN);
            }
            if Some(best.1) == prev {
                chain.pop();
                chain.pop();
                break (a, best.1, best.0);
            }
            chain.push(best.1);
        };
        if a == b || dist.is_nan() {
            break;
        }
        // merge b into a
        let (sa, sb) = (size[a] as f32, size[b] as f32);
        for k in 0..n {
            if active[k] && k != a && k != b {
                let v = (sa * d[a * n + k] + sb * d[b * n + k]) / (sa + sb);
                d[a * n + k] = v;
                d[k * n + a] = v;
            }
        }
        size[a] += size[b];
        active[b] = false;
        left -= 1;
        merges.push(Merge { a: a.min(b), b: a.max(b), d: dist });
        // a's distances changed, so the chain may no longer be valid past it
        chain.retain(|&c| active[c] && c != a);
    }
    merges
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while let Some(&p) = parent.get(x) {
        if p == x {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(x) {
            *slot = gp;
        }
        x = p;
    }
    x
}

/// Cluster labels `0..k` (numbered by first member) after applying merges closer than
/// `threshold`, or exactly the merges that leave `clusters` clusters.
pub fn cut(n: usize, merges: &[Merge], threshold: f32, clusters: Option<usize>) -> Vec<usize> {
    let mut sorted = merges.to_vec();
    sorted.sort_by(|x, y| x.d.total_cmp(&y.d));
    let take = match clusters {
        Some(k) => n.saturating_sub(k.max(1)).min(sorted.len()),
        None => sorted.iter().take_while(|m| m.d <= threshold).count(),
    };
    let mut parent: Vec<usize> = (0..n).collect();
    for m in sorted.iter().take(take) {
        let (ra, rb) = (find(&mut parent, m.a), find(&mut parent, m.b));
        if ra != rb
            && let Some(p) = parent.get_mut(ra.max(rb))
        {
            *p = ra.min(rb);
        }
    }
    let mut ids: Vec<Option<usize>> = vec![None; n];
    let mut next = 0;
    (0..n)
        .map(|i| {
            let r = find(&mut parent, i);
            match ids.get(r).copied().flatten() {
                Some(id) => id,
                None => {
                    if let Some(slot) = ids.get_mut(r) {
                        *slot = Some(next);
                    }
                    next += 1;
                    next - 1
                }
            }
        })
        .collect()
}

/// Unit-length mean of the vectors with each label (`k` labels).
pub fn centroids(points: &[Vec<f32>], labels: &[usize], k: usize, dim: usize) -> Vec<Vec<f32>> {
    let mut c = vec![vec![0f32; dim]; k];
    for (p, &l) in points.iter().zip(labels) {
        if let Some(row) = c.get_mut(l) {
            row.iter_mut().zip(p).for_each(|(a, b)| *a += b);
        }
    }
    c.iter_mut().for_each(|v| normalize(v));
    c
}

/// Index of the nearest centroid.
pub fn nearest(p: &[f32], centroids: &[Vec<f32>]) -> Option<usize> {
    centroids.iter().enumerate().min_by(|x, y| cosine_distance(p, x.1).total_cmp(&cosine_distance(p, y.1))).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        normalize(&mut v);
        v
    }

    #[test]
    fn three_groups_found_by_threshold_and_by_count() {
        let mut pts = Vec::new();
        for i in 0..10 {
            let e = i as f32 * 0.01;
            pts.push(unit(&[1.0, e, 0.0]));
            pts.push(unit(&[e, 1.0, 0.0]));
            pts.push(unit(&[0.0, e, 1.0]));
        }
        let m = average_linkage(&pts);
        assert_eq!(m.len(), pts.len() - 1);
        let l = cut(pts.len(), &m, 0.3, None);
        assert_eq!(*l.iter().max().unwrap(), 2);
        // members of each group share a label
        for g in 0..3 {
            let first = l[g];
            assert!((0..10).all(|i| l[i * 3 + g] == first));
        }
        assert_eq!(l[0], 0);
        let two = cut(pts.len(), &m, 0.3, Some(2));
        assert_eq!(*two.iter().max().unwrap(), 1);
        let one = cut(pts.len(), &m, 0.0, Some(1));
        assert!(one.iter().all(|&x| x == 0));
        let c = centroids(&pts, &l, 3, 3);
        assert_eq!(nearest(&unit(&[0.9, 0.1, 0.0]), &c), Some(l[0]));
    }

    #[test]
    fn degenerate_inputs() {
        assert!(average_linkage(&[]).is_empty());
        assert!(average_linkage(&[vec![1.0]]).is_empty());
        assert_eq!(cut(1, &[], 0.5, None), vec![0]);
        assert_eq!(cut(0, &[], 0.5, Some(3)), Vec::<usize>::new());
        // identical and zero vectors do not loop or panic
        let pts = vec![vec![0.0, 0.0]; 5];
        assert_eq!(average_linkage(&pts).len(), 4);
        assert_eq!(nearest(&[1.0], &[]), None);
    }
}
