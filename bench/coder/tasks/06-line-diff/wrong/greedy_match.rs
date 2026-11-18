// Wrong: valid edit scripts, but not minimal. Each old line is matched with the first
// later occurrence in new, which can skip over lines that a better alignment would keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Equal(String),
    Delete(String),
    Insert(String),
}

pub fn diff(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let mut script = Vec::new();
    let mut j = 0;
    for &line in old {
        match new[j..].iter().position(|&n| n == line) {
            Some(off) => {
                for l in &new[j..j + off] {
                    script.push(Edit::Insert(l.to_string()));
                }
                script.push(Edit::Equal(line.to_string()));
                j += off + 1;
            }
            None => script.push(Edit::Delete(line.to_string())),
        }
    }
    for l in &new[j..] {
        script.push(Edit::Insert(l.to_string()));
    }
    script
}
