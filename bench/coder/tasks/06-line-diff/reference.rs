#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Equal(String),
    Delete(String),
    Insert(String),
}

pub fn diff(old: &[&str], new: &[&str]) -> Vec<Edit> {
    // Trim the common prefix and suffix; the table is only built for what is left.
    let mut pre = 0;
    while pre < old.len() && pre < new.len() && old[pre] == new[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < old.len() - pre && suf < new.len() - pre && old[old.len() - 1 - suf] == new[new.len() - 1 - suf] {
        suf += 1;
    }
    let a = &old[pre..old.len() - suf];
    let b = &new[pre..new.len() - suf];

    let mut script: Vec<Edit> = Vec::with_capacity(old.len() + new.len());
    for l in &old[..pre] {
        script.push(Edit::Equal(l.to_string()));
    }

    // Myers' O((N+M)D) algorithm on the trimmed middle; memory is O(D^2) for the saved frontiers.
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = n + m;
    let off = max + 1;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = 0isize;
    'outer: for d in 0..=max {
        trace.push(v[(off - d) as usize..=(off + d) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]) {
                v[(off + k + 1) as usize]
            } else {
                v[(off + k - 1) as usize] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(off + k) as usize] = x;
            if x >= n && y >= m {
                found = d;
                break 'outer;
            }
            k += 2;
        }
    }
    // Walk back from (n, m), collecting the edits in reverse.
    let mut rev: Vec<Edit> = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=found).rev() {
        let snap = &trace[d as usize]; // frontier before round d; index k + d
        let at = |k: isize| snap[(k + d) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) { k + 1 } else { k - 1 };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            rev.push(Edit::Equal(a[x as usize].to_string()));
        }
        if x == prev_x {
            y -= 1;
            rev.push(Edit::Insert(b[y as usize].to_string()));
        } else {
            x -= 1;
            rev.push(Edit::Delete(a[x as usize].to_string()));
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        rev.push(Edit::Equal(a[x as usize].to_string()));
    }
    rev.reverse();
    script.extend(rev);

    for l in &old[old.len() - suf..] {
        script.push(Edit::Equal(l.to_string()));
    }
    script
}
