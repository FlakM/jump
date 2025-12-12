pub mod github;
pub mod repo;
pub mod types;

pub use github::{GitHubPermalinkGenerator, PermalinkGenerator};
pub use repo::{Git2Repository, GitRepository};
pub use types::{GitHubLink, LineRange, RepoInfo};
