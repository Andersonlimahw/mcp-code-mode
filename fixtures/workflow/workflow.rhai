let found = search(#{ query: "Launch plan" });
let parent_id = found.results[0].id;
let page = create_page(#{
    parent: #{ page_id: parent_id },
    properties: #{
        title: #{
            title: [
                #{
                    type: "text",
                    text: #{ content: "Launch checklist" }
                }
            ]
        }
    }
});
let edited = update_page(#{
    page_id: page.id,
    properties: #{
        title: #{
            title: [
                #{
                    type: "text",
                    text: #{ content: "Launch checklist (edited)" }
                }
            ]
        }
    }
});
let database = create_database(#{
    parent: #{ type: "page_id", page_id: page.id },
    title: [
        #{
            type: "text",
            text: #{ content: "Launch tasks" }
        }
    ],
    properties: #{
        Name: #{ title: #{} }
    }
});
let revised = update_database(#{
    database_id: database.id,
    title: [
        #{
            type: "text",
            text: #{ content: "Launch tasks v2" }
        }
    ],
    description: [
        #{
            type: "text",
            text: #{ content: "Edited by code mode" }
        }
    ]
});
#{
    database_id: revised.id,
    page_id: edited.id,
    parent_id: parent_id,
    steps: ["search", "create_page", "update_page", "create_database", "update_database"]
}
