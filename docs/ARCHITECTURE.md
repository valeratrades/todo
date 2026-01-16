# Sources

## Remote

### Reading from GitHub → Issue

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           GITHUB API RESPONSE                               │
├─────────────────────────────────────────────────────────────────────────────┤
│  GithubIssue {                                                              │
│    number: u64,                                                             │
│    title: String,                                                           │
│    body: Option<String>,                                                    │
│    labels: Vec<GithubLabel>,                                                │
│    user: GithubUser,                                                        │
│    state: String,           // "open" | "closed"                            │
│    state_reason: Option<String>,  // "completed" | "not_planned" | "duplicate"
│    updated_at: String,      // ISO 8601                                     │
│  }                                                                          │
│  + Vec<GithubComment> { id, body, user }                                    │
│  + Vec<GithubIssue>  (sub-issues)                                           │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────┐
                    │      from_github()            │
                    │  (github_sync.rs)             │
                    └───────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────────┐   ┌─────────────────────────┐   ┌─────────────────────┐
│ IssueLink.parse() │   │ CloseState::from_github │   │ split_blockers()    │
│ URL → owner/repo/ │   │ (state, state_reason)   │   │ body → (text,       │
│       number      │   │ → Open/Closed/NotPlanned│   │        BlockerSeq)  │
└───────────────────┘   └─────────────────────────┘   └─────────────────────┘
        │                           │                           │
        ▼                           ▼                           ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Issue struct                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│  meta: IssueMeta {                                                          │
│    title: String,                                                           │
│    identity: IssueIdentity::Linked(IssueLink),                              │
│    close_state: CloseState,                                                 │
│    owned: bool,             // user.login == current_user                   │
│  }                                                                          │
│  labels: Vec<String>,                                                       │
│  comments: Vec<Comment>,    // [0] = body (CommentIdentity::Body)           │
│                             // [1..] = comments (CommentIdentity::Linked)   │
│  children: Vec<Issue>,      // sub-issues, recursively                      │
│  blockers: BlockerSequence, // extracted from body                          │
│  last_contents_change: Option<Timestamp>,  // from updated_at               │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Pushing Issue → GitHub

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Issue struct                                   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────────┐   ┌─────────────────────────┐   ┌─────────────────────┐
│ close_state       │   │ issue.body()            │   │ children/comments   │
│   .to_github_     │   │ = comments[0].body      │   │ with Pending        │
│    state()        │   │ + join_with_blockers()  │   │ identity            │
│   .to_github_     │   │                         │   │                     │
│    state_reason() │   │                         │   │                     │
└───────────────────┘   └─────────────────────────┘   └─────────────────────┘
        │                           │                           │
        ▼                           ▼                           ▼
┌───────────────────┐   ┌─────────────────────────────┐   ┌─────────────────────┐
│ "open"/"closed"   │   │ String (body text           │   │ IssueAction::       │
│ + "completed"/    │   │  with blockers appended)    │   │   CreateIssue {     │
│   "not_planned"   │   │                             │   │     title, body,    │
│                   │   │                             │   │     closed, parent  │
│                   │   │                             │   │   }                 │
└───────────────────┘   └─────────────────────────────┘   └─────────────────────┘
        │                           │                           │
        └───────────────────────────┼───────────────────────────┘
                                    ▼
                    ┌───────────────────────────────┐
                    │   sync_local_issue_to_github  │
                    │   (sync.rs)                   │
                    └───────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────────┐   ┌─────────────────────────┐   ┌─────────────────────┐
│ gh.update_issue_  │   │ gh.update_issue_body()  │   │ gh.create_issue()   │
│   state()         │   │ gh.create_comment()     │   │ gh.add_sub_issue()  │
│                   │   │ gh.update_comment()     │   │                     │
│                   │   │ gh.delete_comment()     │   │                     │
└───────────────────┘   └─────────────────────────┘   └─────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           GITHUB API CALLS                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│  PATCH /repos/{owner}/{repo}/issues/{number}                                │
│    { "state": "open"|"closed", "state_reason": "...", "body": "..." }       │
│  POST  /repos/{owner}/{repo}/issues/{number}/comments  { "body": "..." }    │
│  POST  /repos/{owner}/{repo}/issues  { "title": "...", "body": "..." }      │
│        → CreatedIssue { id, number, html_url }                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Type Locations

| Type | File |
|------|------|
| `Issue`, `IssueMeta`, `CloseState`, `IssueIdentity` | `src/issue/types.rs` |
| `GithubIssue`, `GithubComment`, `CreatedIssue` | `src/github.rs` |
| `from_github()` | `src/open_interactions/github_sync.rs` |
| `split_blockers()`, `join_with_blockers()` | `src/issue/blocker.rs` |
| `sync_local_issue_to_github()` | `src/open_interactions/sync.rs` |
