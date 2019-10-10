use seed::prelude::*;
use serde::{Deserialize, Serialize};
use tap::*;

use crate::{
    locations::Location,
    messages::{AsyncM as GlobalAsyncM, M as GlobalM},
    model::{
        PostMarker, Store as GlobalS, StoreOpResult as GSOpResult, StoreOperations as GSOp, User,
    },
};
use db_models::models::*;

pub fn load_post(post_marker: PostMarker) -> impl GlobalAsyncM {
    use seed::fetch::Request;
    const POSTS_URL: &str = "/api/posts";
    let url = format!("{}/{}", POSTS_URL, post_marker);
    Request::new(url)
        .fetch_json(move |fo| GlobalM::StoreOpWithAction(GSOp::Post(post_marker, fo), after_fetch))
}
fn after_fetch(gs: *const GlobalS, res: GSOpResult) -> Option<GlobalM> {
    use GSOpResult::*;
    let gs = unsafe { gs.as_ref() }?;
    match (res, &gs.post) {
        (Success, Some(post)) => Some(GlobalM::RenderPage(Location::Editor(S::Old(post.clone())))),
        _ => None,
    }
}
pub fn is_restricted_from(s: &S, gs: &GlobalS) -> bool {
    if let GlobalS {
        user: Some(user), ..
    } = gs
    {
        match s {
            S::Old(stored_post) => !stored_post.is_published() && !user.can_see_unpublished,
            S::New(_) => false,
            S::Undetermined(_) => false,
        }
    } else {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum M {
    Title(String),
    Body(String),
    Slug(String),
    Publish,
    Save,

    SyncPost,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S {
    Undetermined(PostMarker),
    New(posts::NewNoMeta),
    Old(posts::DataNoMeta),
}
impl From<PostMarker> for S {
    fn from(s: PostMarker) -> Self {
        Self::Undetermined(s)
    }
}
impl S {
    pub fn to_url(&self) -> Url {
        use S::*;
        let id = match self {
            New(_) => "new".to_owned(),
            Old(post) => (post.into(): PostMarker).to_string(),
            Undetermined(pm) => pm.to_string(),
        };
        Url::new(vec!["blog", "edit", id.as_str()])
    }
    pub fn is_loaded(&self) -> bool {
        if let Self::Undetermined(_) = self {
            false
        } else {
            true
        }
    }
    pub fn is_publishable(&self) -> bool {
        match self {
            Self::New(_) => true,
            Self::Old(post) => match post {
                // If not published, or archived but not deleted, allow publish button.
                posts::DataNoMeta {
                    published_at: None,
                    archived_at: None,
                    deleted_at: None,
                    ..
                }
                | posts::DataNoMeta {
                    archived_at: Some(_),
                    deleted_at: None,
                    ..
                } => true,
                _ => false,
            },
            Self::Undetermined(_) => false,
        }
    }
    pub fn old_ref(&self) -> Option<&posts::DataNoMeta> {
        match self {
            Self::Old(p) => Some(p),
            _ => None,
        }
    }
}
impl Default for S {
    fn default() -> Self {
        Self::New(posts::NewNoMeta::default())
    }
}

impl S {
    fn attempt_save(&mut self) -> Option<Box<dyn GlobalAsyncM>> {
        use seed::fetch::{Method, Request};
        let (url, method) = match self {
            Self::Undetermined(_) => None,
            Self::New(_) => {
                const CREATE_POST_URL: &str = "/api/posts";
                let create_post_method = Method::Post;
                // save
                Some((CREATE_POST_URL.to_owned(), create_post_method))
            }
            Self::Old(post) => {
                const UPDATE_POST_BASE_URL: &str = "/api/posts";
                let update_post_method = Method::Patch;
                Some((
                    format!("{}/{}", UPDATE_POST_BASE_URL, post.id),
                    update_post_method,
                ))
            }
        }?;
        let req = Request::new(url).method(method);
        if let Self::New(post) = self {
            // save
            let followup = |_gs, res| {
                use crate::model::StoreOpResult::*;
                match res {
                    Success => {
                        log::debug!("Post is saved! Modifying state to be `Old` instead of `New`");
                        Some(GlobalM::Editor(M::SyncPost))
                    }
                    Failure(e) => {
                        log::error!("Post save failed due to {:?}.", e);
                        None
                    }
                }
            };
            let reaction =
                move |fo| GlobalM::StoreOpWithAction(GSOp::PostWithoutMarker(fo), followup);
            Some(Box::new(req.send_json(post).fetch_json(reaction)))
        } else if let Self::Old(post) = self {
            let replacing_post = post.clone();
            let reaction = move |res: Result<_, _>| match res
                .tap_ok(|_| log::debug!("Launching credential creation"))
                .tap_err(|e| log::error!("Post save failed due to {:?}.", e))
            {
                Ok(_) => GlobalM::StoreOp(GSOp::PostRaw(replacing_post)),
                Err(_) => GlobalM::NoOp,
            };
            Some(Box::new(req.send_json(post).fetch_string_data(reaction)))
        } else {
            None
        }
    }
    fn attempt_publish(&mut self, user: &User) -> Option<Box<dyn GlobalAsyncM>> {
        use seed::fetch::{Method, Request};
        match self {
            Self::Undetermined(_) => None,
            Self::New(post) => {
                const CREATE_POST_URL: &str = "/api/posts";
                post.published_at = Some(chrono::Utc::now());
                post.published_by = Some(user.id);
                // save
                let (url, method) = (CREATE_POST_URL.to_string(), Method::Post);
                let followup = |gs: *const GlobalS, res| {
                    let gs = unsafe { gs.as_ref() }?;
                    if let (GSOpResult::Success, Some(posts::DataNoMeta { id: post_id, .. })) =
                        (res, &gs.post)
                    {
                        Some(GlobalM::ChangePageAndUrl(Location::Viewer(
                            PostMarker::Uuid(*post_id).into(),
                        )))
                    } else {
                        None
                    }
                };
                let reaction =
                    move |fo| GlobalM::StoreOpWithAction(GSOp::PostWithoutMarker(fo), followup);
                let req = Request::new(url)
                    .method(method)
                    .send_json(post)
                    .fetch_json(reaction);
                Some(Box::new(req))
            }
            Self::Old(post) => {
                let post_id = post.id;
                let (url, method) = (format!("/api/posts/{}/publish", post.id), Method::Post);
                let reaction = move |res| match res {
                    Ok(_) => GlobalM::ChangePageAndUrl(Location::Viewer(
                        PostMarker::Uuid(post_id).into(),
                    )),
                    _ => GlobalM::NoOp,
                };
                let req = Request::new(url).method(method).fetch_string_data(reaction);
                Some(Box::new(req))
            }
        }
    }
}

fn update_post(to_update: &mut posts::DataNoMeta, updated: &posts::DataNoMeta) {
    to_update.created_by = updated.created_by;
    to_update.created_at = updated.created_at;
    to_update.published_by = updated.published_by;
    to_update.published_at = updated.published_at;
    to_update.archived_by = updated.archived_by;
    to_update.archived_at = updated.archived_at;
    to_update.deleted_by = updated.deleted_by;
    to_update.deleted_at = updated.deleted_at;
    to_update.title = updated.title.clone();
    to_update.body = updated.body.clone();
    to_update.slug = updated.slug.clone();
}
pub fn update(m: M, s: &mut S, gs: &GlobalS, orders: &mut impl Orders<M, GlobalM>) {
    use M::*;
    let (post_title, post_body, post_slug) = match s {
        S::New(post) => (&mut post.title, &mut post.body, &mut post.slug),
        S::Old(post) => (&mut post.title, &mut post.body, &mut post.slug),
        S::Undetermined(_) => return,
    };
    match m {
        Title(title) => {
            *post_title = title;
        }
        Body(body) => {
            *post_body = body;
        }
        Slug(slug) => match slug.trim() {
            "" => *post_slug = None,
            _ => *post_slug = Some(slug),
        },
        Publish => {
            gs.user
                .as_ref()
                .and_then(|u| s.attempt_publish(u))
                .map(|req| orders.perform_g_cmd(req));
        }
        Save => {
            s.attempt_save().map(|req| orders.perform_g_cmd(req));
        }

        SyncPost => {
            if let Some(updated) = &gs.post {
                match s {
                    S::Old(post) if post.id == updated.id => update_post(post, updated),
                    _ => {
                        orders.send_g_msg(GlobalM::ChangePageAndUrl(Location::Editor(S::Old(
                            updated.clone(),
                        ))));
                    }
                }
            }
        }
    }
}

pub use views::render;
mod views {
    use seed::prelude::*;

    use super::{M, S};
    use crate::model::Store as GlobalS;

    pub fn render(s: &S, _gs: &GlobalS) -> Vec<Node<M>> {
        vec![
            heading(),
            editor(s).unwrap_or_else(crate::shared::views::loading),
        ]
    }

    pub fn heading() -> Node<M> {
        h1![attrs! { At::Class => "as-h3" }, "Editing"]
    }

    fn get_title_slug_body(s: &S) -> Option<(&str, Option<&str>, &str)> {
        let (t, slug, b) = match s {
            S::New(post) => (&post.title, post.slug.as_ref(), &post.body),
            S::Old(post) => (&post.title, post.slug.as_ref(), &post.body),
            _ => return None,
        };
        let slug = slug.map(String::as_str);
        Some((t, slug, b))
    }
    fn title_field(title: &str) -> Node<M> {
        div![
            attrs! { At::Class => "editor-title" },
            input![
                {
                    let mut attrs = attrs! {
                        At::Placeholder => "Title";
                        At::Type => "text";
                        At::Name => "title",
                        At::Value => title,
                    };
                    attrs.add_multiple(At::Class, &["single-line-text-entry", "as-h1"]);
                    attrs
                },
                input_ev(Ev::Input, M::Title),
            ],
        ]
    }
    fn slug_field(slug: &str, hint: &str) -> Node<M> {
        div![
            attrs! { At::Class => "editor-slug" },
            label![
                {
                    let mut attrs = attrs! {
                        At::For => "slug",
                    };
                    attrs.add_multiple(At::Class, &["same-line-label", "as-pre"]);
                    attrs
                },
                "/blog/posts/",
            ],
            input![
                {
                    let mut attrs = attrs! {
                        At::Placeholder => hint;
                        At::Type => "text";
                        At::Name => "slug",
                        At::Value => slug,
                    };
                    attrs.add_multiple(At::Class, &["single-line-text-entry", "as-pre"]);
                    attrs
                },
                input_ev(Ev::Input, M::Slug),
            ],
        ]
    }
    fn body_field(body: &str) -> Node<M> {
        div![
            attrs! {
                At::Class => "editor-body",
            },
            textarea![
                {
                    let mut attrs = attrs! {
                        At::Placeholder => "Write your post here!";
                        At::Type => "text";
                        At::Name => "body",
                    };
                    attrs.add_multiple(At::Class, &["multi-line-text-entry"]);
                    attrs
                },
                body,
                input_ev(Ev::Input, M::Body),
            ],
        ]
    }
    fn action_buttons(s: &S) -> Node<M> {
        div![
            attrs! {
                At::Class => "editor-actions",
            },
            input![
                attrs! {
                    At::Class => "inline-button",
                    At::Type => "submit",
                    At::Value => "Save",
                },
                raw_ev(Ev::Click, |e| {
                    e.prevent_default();
                    M::Save
                }),
            ],
            if s.is_publishable() {
                input![
                    attrs! {
                        At::Class => "inline-button",
                        At::Type => "submit",
                        At::Value => "Publish",
                    },
                    raw_ev(Ev::Click, |e| {
                        e.prevent_default();
                        M::Publish
                    }),
                ]
            } else {
                empty![]
            },
        ]
    }
    pub fn editor(s: &S) -> Option<Node<M>> {
        let (title, slug, body) = get_title_slug_body(s)?;
        let slug_hint_mem = slug
            .map(|_| None)
            .unwrap_or_else(|| s.old_ref().map(|p| p.id.to_hyphenated_ref().to_string()));
        let slug_hint = slug_hint_mem
            .as_ref()
            .map(String::as_str)
            .or(slug)
            .unwrap_or("");
        Some(div![
            attrs! { At::Class => "editor" },
            title_field(title),
            slug_field(slug.unwrap_or(""), slug_hint),
            body_field(body),
            action_buttons(s),
        ])
    }
}
