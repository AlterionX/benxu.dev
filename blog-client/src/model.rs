use seed::fetch::FetchObject;
use seed::prelude::*;
use serde::{Deserialize, Serialize};
use tap::*;

use crate::{locations::*, messages::M, requests};
use db_models::models::{posts, users};

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Name {
    pub first: String,
    pub last: String,
    pub nickname: String,
}
impl Name {
    pub fn to_view<M: Clone>(&self) -> seed::dom_types::Node<M> {
        p![format!("By {} {}", self.first, self.last)]
    }
}
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct User {
    pub id: uuid::Uuid,
    pub name: Name,
    pub can_see_unpublished: bool,
}
impl From<users::DataNoMeta> for User {
    fn from(u: users::DataNoMeta) -> User {
        Self {
            id: u.id,
            name: crate::model::Name {
                first: u.first_name.unwrap_or_else(|| "unknown".to_owned()),
                last: u.last_name.unwrap_or_else(|| "unknown".to_owned()),
                nickname: "unknown".to_owned(),
            },
            can_see_unpublished: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PostMarker {
    Uuid(uuid::Uuid),
    Slug(String),
}
impl PostMarker {
    fn create_slug(slug: &str) -> Self {
        Self::Slug(slug.to_owned())
    }
    pub fn refers_to(&self, post: &posts::DataNoMeta) -> bool {
        match self {
            Self::Uuid(id) => id == &post.id,
            Self::Slug(s) => Some(s) == post.slug.as_ref(),
        }
    }
}
impl From<String> for PostMarker {
    fn from(s: String) -> Self {
        uuid::Uuid::parse_str(s.as_str()).map_or_else(|_| Self::Slug(s), Self::Uuid)
    }
}
impl From<&str> for PostMarker {
    fn from(s: &str) -> Self {
        uuid::Uuid::parse_str(s).map_or_else(|_| Self::create_slug(s), Self::Uuid)
    }
}
impl From<&posts::DataNoMeta> for PostMarker {
    fn from(post: &posts::DataNoMeta) -> Self {
        if let Some(slug) = &post.slug {
            PostMarker::from(slug.clone())
        } else {
            PostMarker::Uuid(post.id)
        }
    }
}
impl std::fmt::Display for PostMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_slug())
    }
}
impl PostMarker {
    fn to_slug(&self) -> String {
        match self {
            Self::Uuid(u) => u.to_hyphenated_ref().to_string(),
            Self::Slug(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StoreOperations {
    Post(PostMarker, FetchObject<posts::DataNoMeta>),
    PostWithoutMarker(FetchObject<posts::DataNoMeta>),
    PostRaw(posts::DataNoMeta),
    PostListing(requests::PostQuery, FetchObject<Vec<posts::BasicData>>),
    User(FetchObject<users::DataNoMeta>),
    RemoveUser(FetchObject<String>),
}
impl PartialEq for StoreOperations {
    fn eq(&self, rhs: &StoreOperations) -> bool {
        match (self, rhs) {
            (Self::Post(lhs, _), Self::Post(rhs, _)) => lhs == rhs,
            (Self::PostRaw(lhs), Self::PostRaw(rhs)) => lhs == rhs,
            (Self::PostWithoutMarker(_), Self::PostWithoutMarker(_)) => false,
            (Self::PostListing(lhs, _), Self::PostListing(rhs, _)) => lhs == rhs,
            (Self::RemoveUser(_), Self::RemoveUser(_)) => true,
            _ => false,
        }
    }
}
impl Eq for StoreOperations {}
impl std::hash::Hash for StoreOperations {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Post(q, _) => q.hash(state),
            Self::PostRaw(p) => p.hash(state),
            Self::PostListing(q, _) => q.hash(state),
            Self::User(_) => (),
            Self::RemoveUser(_) => (),
            Self::PostWithoutMarker(_) => (),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FailReason {
    Req,
    Data { is_dom_err: bool },
    Status(u16, String),
}
impl<T> From<seed::fetch::FailReason<T>> for FailReason {
    fn from(e: seed::fetch::FailReason<T>) -> Self {
        match e {
            seed::fetch::FailReason::RequestError(_, _) => Self::Req,
            seed::fetch::FailReason::Status(s, _) => Self::Status(s.code, s.text),
            seed::fetch::FailReason::DataError(e, _) => Self::Data {
                is_dom_err: if let seed::fetch::DataError::DomException(_) = e {
                    true
                } else {
                    false
                },
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StoreOpResult {
    Success,
    Failure(FailReason),
}
impl From<Result<(), FailReason>> for StoreOpResult {
    fn from(res: Result<(), FailReason>) -> Self {
        match res {
            Ok(_) => Self::Success,
            Err(e) => Self::Failure(e),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Store {
    pub published_posts: Option<Vec<posts::BasicData>>,
    pub unpublished_posts: Option<Vec<posts::BasicData>>,
    pub post: Option<posts::DataNoMeta>,
    pub user: Option<User>,
}
impl Store {
    pub fn with_user(user: users::DataNoMeta) -> Self {
        Self {
            user: Some(user.into()),
            ..Self::default()
        }
    }
    pub fn exec(&mut self, op: StoreOperations) -> Result<(), FailReason> {
        use StoreOperations::*;
        match op {
            PostListing(_q, fetched) => {
                log::trace!("Post listing store operation triggered.");
                // TODO use query data to implement cache.
                let fetched = fetched.response()?;
                let mut available_posts: Vec<_> = fetched
                    .data
                    .into_iter()
                    .filter(|post| post.deleted_at.is_none())
                    .collect();
                let published = available_posts
                    .drain_filter(|post| post.is_published())
                    .collect();
                let unpublished = available_posts;
                self.published_posts.replace(published);
                self.unpublished_posts.replace(unpublished);
            }
            RemoveUser(fo) => {
                log::trace!("User clear operation triggered.");
                fo.response().tap_err(|e| {
                    log::warn!("Error {:?} occurred! TODO: show an error to the user.", e)
                })?;
                self.user = None;
            }
            User(fo) => {
                log::trace!("User store operation triggered.");
                let fetched = fo.response().tap_err(|e| {
                    log::warn!("Error {:?} occurred! TODO: show an error to the user.", e)
                })?;
                let unparsed = fetched.data;
                let parsed = unparsed.into();
                self.user.replace(parsed);
            }
            Post(_, fo) | PostWithoutMarker(fo) => {
                let fetched = fo.response().tap_err(|e| {
                    log::warn!("Error {:?} occurred! TODO: show an error to the user.", e)
                })?;
                self.post.replace(fetched.data);
            }
            PostRaw(raw_post) => {
                self.post.replace(raw_post);
            }
        }
        Ok(())
    }
    pub fn has_cached_post(&self, id: &PostMarker) -> bool {
        use PostMarker::*;
        match (&self.post, &id) {
            (Some(db_models::posts::DataNoMeta { id: cached_id, .. }), Uuid(id)) => {
                *id == *cached_id
            }
            (
                Some(db_models::posts::DataNoMeta {
                    slug: Some(cached_slug),
                    ..
                }),
                Slug(slug),
            ) => *slug == *cached_slug,
            _ => false,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Model {
    pub store: Store,
    pub loc: Location,
}
impl Model {
    pub fn with_user(user: users::DataNoMeta) -> Self {
        Self {
            store: Store::with_user(user),
            loc: Location::NotFound,
        }
    }
    pub fn to_view(&self) -> Vec<Node<M>> {
        log::info!(
            "Rendering location {:?} with global state {:?}.",
            self.loc,
            self.store
        );
        self.loc.to_view(&self.store)
    }
}
