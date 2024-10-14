use askama::Template;
use octocrab::Octocrab;
use tokio::fs::{self, File};
use tokio::io;
use walkdir::WalkDir;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (owner, repo) = ("honnip", "website");
    let dist = "output";
    let asset = "assets";

    // output dir
    fs::remove_dir_all(dist).await.or_else(ignore_not_found)?;
    fs::create_dir(dist).await?;
    fs::create_dir(format!("{dist}/posts")).await?;

    // copy the assets
    for entry in WalkDir::new(asset) {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }

        let path = entry.path();
        let dest = format!("{dist}/{}", path.strip_prefix(asset)?.display());

        if path.is_dir() {
            fs::create_dir(dest).await?;
        } else {
            fs::copy(path, dest).await?;
        }
    }

    // fetch all posts
    let posts = fetch_posts(owner, repo).await?;

    // create index page
    create_html(
        "output/index.html",
        IndexTemplate {
            posts: &posts,
            owner: &Author {
                name: owner.to_string(),
                avatar: format!(
                    "https://avatars.githubusercontent.com/u/{}?v=4&s=100",
                    std::env::var("GITHUB_REPOSITORY_OWNER_ID")?
                ),
            },
        },
    )
    .await?;

    // create about page
    create_html(
        "output/about.html",
        AboutTemplate {
            owner: &Author {
                name: owner.to_string(),
                avatar: format!(
                    "https://avatars.githubusercontent.com/u/{}?v=4&s=100",
                    std::env::var("GITHUB_REPOSITORY_OWNER_ID")?
                ),
            },
        },
    )
    .await?;

    // create posts page
    create_html(
        "output/posts.html",
        PostsTemplate {
            posts: &posts,
            owner: &Author {
                name: "Honnip".to_string(),
                avatar: format!(
                    "https://avatars.githubusercontent.com/u/{}?v=4&s=100",
                    std::env::var("GITHUB_REPOSITORY_OWNER_ID")?
                ),
            },
        },
    )
    .await?;

    // create post page
    for post in posts {
        if post.status == PostStatus::Draft {
            continue;
        }

        let path = format!("output/posts/{}.html", post.slug);
        create_html(
            path,
            PostTemplate {
                post: &post,
                author: &post.author,
                owner: &Author {
                    name: owner.to_string(),
                    avatar: format!(
                        "https://avatars.githubusercontent.com/u/{}?v=4&s=100",
                        std::env::var("GITHUB_REPOSITORY_OWNER_ID")?
                    ),
                },
            },
        )
        .await?;
    }

    Ok(())
}

async fn fetch_posts(owner: &str, repo: &str) -> anyhow::Result<Vec<Post>> {
    let token =
        std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable is required");

    let octocrab = Octocrab::builder().personal_token(token).build()?;

    let mut articles = Vec::new();
    let mut cursor: Option<String> = None;
    let mut has_next_page: bool = false;

    // fetch all discussions
    loop {
        let discussions: serde_json::Value = octocrab
            .graphql(&serde_json::json!({ "query": generate_query(owner, repo, cursor.as_deref())}))
            .await
            .expect("Failed to fetch discussions");

        for discussion in discussions["data"]["repository"]["discussions"]["edges"]
            .as_array()
            .expect("Not expected format. API changed?")
        {
            println!("{:#?}\n", discussion);
            cursor = Some(discussion["cursor"].as_str().unwrap().to_string());
            let node = discussion["node"].as_object().unwrap();
            let author = node["author"].as_object().unwrap();
            let category = node["category"].as_object().unwrap();
            let labels = node["labels"]["edges"].as_array().unwrap();
            has_next_page = discussions["data"]["repository"]["discussions"]["pageInfo"]
                ["hasNextPage"]
                .as_bool()
                .unwrap();

            let mut labels_vec = Vec::new();
            for label in labels {
                let node = label["node"].as_object().unwrap();
                labels_vec.push(Label {
                    name: node["name"].as_str().unwrap().to_string(),
                    description: node["description"].as_str().map(|s| s.to_string()),
                    color: node["color"].as_str().unwrap().to_string(),
                });
            }

            let title = node["title"].as_str().unwrap().to_string();
            let (title, slug) = title
                .split_once("#")
                .expect("Discussion title should contain a slug. Format: My title#slug");

            articles.push(Post {
                title: title.to_string(),
                slug: slug.to_string(),
                body: node["bodyHTML"].as_str().unwrap().to_string(),
                author: Author {
                    name: author["login"].as_str().unwrap().to_string(),
                    avatar: author["avatarUrl"].as_str().unwrap().to_string() + "&s=100",
                },
                status: if category["name"] == "Published" {
                    PostStatus::Published
                } else {
                    PostStatus::Draft
                },
                published_at: node["createdAt"]
                    .as_str()
                    .unwrap()
                    .split('T')
                    .next()
                    .unwrap()
                    .to_string(),
                updated_at: node["updatedAt"]
                    .as_str()
                    .unwrap()
                    .split('T')
                    .next()
                    .unwrap()
                    .to_string(),
                labels: labels_vec,
            });
        }
        if !has_next_page {
            break;
        }
    }

    Ok(articles)
}

fn generate_query(owner: &str, repo: &str, cursor: Option<&str>) -> String {
    let mut after = String::new();
    if let Some(cursor) = cursor {
        after = format!(r#"after: "{}","#, cursor);
    }

    let query = format!(
        r#"{{
            repository(owner: "{owner}", name: "{repo}") {{
                discussions(first: 1, {after} orderBy: {{ field: CREATED_AT, direction: DESC }} ) {{
                    edges {{
                        cursor
                        node {{
                            title
                            createdAt
                            updatedAt
                            databaseId
                            bodyHTML
                            author {{
                                login
                                avatarUrl
                            }}
                            category {{ name }}
                            labels(first: 10) {{
                                edges {{
                                    node {{
                                        name
                                        description
                                        color
                                    }}
                                }}
                            }}
                        }}
                    }}
                    pageInfo {{
                        hasNextPage
                    }}
                }}
            }}
        }}"#
    );

    query
}

#[derive(Template)]
#[template(path = "index.html", escape = "none", whitespace = "suppress")]
struct IndexTemplate<'a> {
    posts: &'a Vec<Post>,
    owner: &'a Author,
}

#[derive(Template)]
#[template(path = "about.html", escape = "none", whitespace = "suppress")]
struct AboutTemplate<'a> {
    owner: &'a Author,
}

#[derive(Template)]
#[template(path = "posts.html", escape = "none", whitespace = "suppress")]
struct PostsTemplate<'a> {
    posts: &'a Vec<Post>,
    owner: &'a Author,
}

#[derive(Template)]
#[template(path = "post.html", escape = "none", whitespace = "suppress")]
struct PostTemplate<'a> {
    post: &'a Post,
    author: &'a Author,
    owner: &'a Author,
}

struct Post {
    title: String,
    slug: String,
    body: String,
    author: Author,
    status: PostStatus,
    /// yyyy-mm-dd
    published_at: String,
    /// yyyy-mm-dd
    updated_at: String,
    labels: Vec<Label>,
}

#[derive(PartialEq)]
enum PostStatus {
    Draft,
    Published,
}

struct Author {
    name: String,
    /// URL
    avatar: String,
}

struct Label {
    name: String,
    description: Option<String>,
    /// Hex color code without #
    color: String,
}

fn ignore_not_found(e: io::Error) -> io::Result<()> {
    if e.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(e)
    }
}

async fn create_html(
    path: impl AsRef<std::path::Path>,
    template: impl Template,
) -> anyhow::Result<()> {
    let mut file = File::create(path).await?.into_std().await;
    template.write_into(&mut file)?;
    Ok(())
}
