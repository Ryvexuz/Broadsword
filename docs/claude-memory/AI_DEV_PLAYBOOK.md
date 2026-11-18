# AI development playbook

Lessons learned building software with AI agents. They apply to any project. Use them when planning, briefing, reviewing,
testing and running agents in parallel.

Source: the "Portable lessons" section of RETRO.md in Lazlo's Broadsword repo. Do not edit this copy by hand: change
RETRO.md and run `python -I scripts/sync_playbook.py` there. Tags such as [S3] name the Broadsword stage a lesson came from.

**Planning and briefs**
1. **Small, exact tasks are the biggest cost lever.** Name the files, the API and the done check. Broadsword coder passes dropped from 250–400k tokens to about 72k this way. [S3]
2. **Name the hard cases in the spec up front:** what must never happen, the edge inputs and the security boundaries. Longer specs meant fewer review rounds. [S2]
3. **A list in a brief is either marked "for example" or is the complete rule.** Models read an unmarked list as exhaustive. [S3]
4. **Write the done check as commands with the output each must show,** in words that can only be read one way. Coder, reviewer and orchestrator then run the same check. If it can still be read two ways, check both before claiming done. [S3]
5. **Keep a brief template and a coder-prompt template.** The rules every task needs (claim, build route, what never to do, what to report) are then written once, not retyped and forgotten. [S3]

**Review**
6. **An independent reviewer on a strong model pays for itself.** Expect the first pass to fail; plan for two. Every first-pass failure was a real bug the coder's own tests had passed. [S1]
7. **Use a cheap reviewer for low-risk work and a strong one for risky code** such as locks, deletion and secrets. A cheap review costs a fraction of a cent, so reviewing everything is affordable. [S3]
8. **A reviewer working from the diff alone misses runtime bugs.** Give it the test output too. [S3]
9. **When a finding breaks a rule that must always hold, enforce the rule in one place,** not only in the reported case. [S1]

**Tests**
10. **Prove a new test can fail.** Run it once without the fix; a test that can't fail is worse than none. [S1, S3]
11. **A test seam built into the real program may only make a guard stricter,** never weaker. [S1]

**Running agents in parallel**
12. **Scaling up exposes blast radius before throughput.** First ask what one agent's failure can stop for everyone. [S3]
13. **Shared state needs real OS locks and isolated work areas:** one branch or worktree per agent, claimed files, and one owner for merging. [S3]
14. **A tool that agents call must survive their mistakes,** such as closed pipes, killed processes and bad input, without leaving shared state broken. [S3]
15. **Check that every setting is actually used.** A limit nobody reads looks like safety and isn't. [S3]

**Security**
16. **Secrets leak through side channels,** such as a child's environment or the same value under another name. Protect a secret where it enters a process, not where it is printed. [S2]

**Tools an AI uses**
17. **Test a tool surface with a real model as the client.** It follows the descriptions literally, into dead ends that protocol tests miss. [S2]

**Process**
18. **A fix counts only once something enforces it:** code, a role file or a checklist. A note alone is forgotten by the next session. [S3]
19. **Push long runs, such as full test suites and reviews, into the background,** and keep working. The orchestrator is the bottleneck, not the coders. [S3]
20. **Track whether each process change helps or hurts,** with a measure and a watch-for-harm signal, and revert the ones that hinder. A process only gets lighter if changes can be taken back out. [S3]
21. **A subagent cannot wait for its own background jobs.** Tell it to run long checks in the foreground and to read the result before it reports. Otherwise it hands back unverified work. [S3]
22. **Enforce an environment rule where the process is spawned,** not in each command builder. A rule set in two builders was missed by a third path. [S3]
23. **Write code files with an editor tool, not through shell heredocs.** Shell quoting silently changes backslashes and quotes. [S3]
24. **Audit the whole project at every stage end, with a strong model that didn't build it.** Per-change reviews only see their diff, while the worst problems sit between features and rules. Fix the audit's blockers before tagging. [S3 audit]
25. **Check every new feature against the standing rules it touches,** especially secrets, deletion and settings. A documented feature can quietly break a higher rule. [S3 audit]
26. **Records drift toward over-claiming.** Write them from raw evidence and quote it, and have the audit check the claims. [S3 audit]
27. **No exemptions for the orchestrator.** The strongest model is the one most tempted to skip the process it enforces. [S3 audit]
28. **A rule without a mechanism is a wish.** Enforce each standing rule in code, a hook or a test, or label it "not yet in force". [S3 audit]
