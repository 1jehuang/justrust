use std::collections::{HashSet, VecDeque, BinaryHeap, HashMap};
use std::cmp::Reverse;
const MAP: &str = "\
S..#......
.#.#.####.
.#...#....
.####.#.#.
......#.#E
.#####..#.
.....#.##.
";
fn neighbors(g: &[Vec<u8>], (r, c): (usize, usize)) -> impl Iterator<Item = (usize, usize)> + '_ {
    [(0i32, 1i32), (1, 0), (0, -1), (-1, 0)].into_iter().filter_map(move |(dr, dc)| { let (nr, nc) = (r as i32 + dr, c as i32 + dc); if nr < 0 || nc < 0 { return None; } let (nr, nc) = (nr as usize, nc as usize); (nr < g.len() && nc < g[0].len() && g[nr][nc] != b'#').then_some((nr, nc)) })
}
fn find(g: &[Vec<u8>], ch: u8) -> (usize, usize) { g.iter().enumerate().find_map(|(r, row)| row.iter().position(|&x| x == ch).map(|c| (r, c))).unwrap() }
fn main() {
    let g: Vec<Vec<u8>> = MAP.lines().map(|l| l.bytes().collect()).collect();
    let (s, e) = (find(&g, b'S'), find(&g, b'E'));
    let mut dist = HashMap::new(); dist.insert(s, 0); let mut q = VecDeque::from([s]);
    while let Some(p) = q.pop_front() { for n in neighbors(&g, p) { if !dist.contains_key(&n) { dist.insert(n, dist[&p] + 1); q.push_back(n); } } }
    println!("bfs {:?} reachable={}", dist.get(&e), dist.len());
    let cost = |(r, c): (usize, usize)| 1 + (r * 7 + c * 3) % 5;
    let mut best: HashMap<(usize, usize), usize> = HashMap::new(); let mut h = BinaryHeap::new(); h.push(Reverse((0usize, s)));
    while let Some(Reverse((d, p))) = h.pop() { if best.get(&p).is_some_and(|&b| b <= d) { continue; } best.insert(p, d); if p == e { break; } for n in neighbors(&g, p) { h.push(Reverse((d + cost(n), n))); } }
    println!("dijkstra {}", best[&e]);
    let mut seen = HashSet::new(); let mut regions = 0;
    for r in 0..g.len() { for c in 0..g[0].len() { if g[r][c] == b'#' && seen.insert((r, c)) { regions += 1; let mut st = vec![(r, c)]; while let Some((a, b)) = st.pop() { for (dr, dc) in [(0i32, 1i32), (1, 0), (0, -1), (-1, 0)] { let (x, y) = (a as i32 + dr, b as i32 + dc); if x >= 0 && y >= 0 && (x as usize) < g.len() && (y as usize) < g[0].len() && g[x as usize][y as usize] == b'#' && seen.insert((x as usize, y as usize)) { st.push((x as usize, y as usize)); } } } } } }
    println!("wall regions {regions}");
}
