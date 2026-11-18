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

    // lcs[i][j] = length of the longest common subsequence of a[i..] and b[j..]
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let mut lcs = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * w + j] = if a[i] == b[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    // WRONG: stops when old is used up, so lines that new adds at the end are lost
    while i < n {
        if i < n && j < m && a[i] == b[j] {
            script.push(Edit::Equal(a[i].to_string()));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            script.push(Edit::Delete(a[i].to_string()));
            i += 1;
        } else {
            script.push(Edit::Insert(b[j].to_string()));
            j += 1;
        }
    }

    for l in &old[old.len() - suf..] {
        script.push(Edit::Equal(l.to_string()));
    }
    script
}
