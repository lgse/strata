use super::*;

#[test]
fn the_planner_follows_strata() {
    let cwd = format!("{HOME}/Projects");
    let labels = |t: &str| -> Vec<String> {
        suggest(t, &cwd)
            .0
            .into_iter()
            .map(|c| c.replacement)
            .collect()
    };
    assert_eq!(
        labels("~/D"),
        ["~/Desktop/", "~/Documents/", "~/Downloads/"]
    );
    assert_eq!(labels("~"), ["~/"]);
    assert_eq!(labels("~/.c"), ["~/.config/"]);
    // A folder that matches alone lists its children instead.
    assert_eq!(
        labels("~/Documents"),
        [
            "~/Documents/Invoices/",
            "~/Documents/Letters/",
            "~/Documents/Notes/"
        ]
    );
    assert_eq!(
        labels("str"),
        [
            format!("{cwd}/strata-components/"),
            format!("{cwd}/strata/")
        ]
    );
    assert_eq!(suggest("~/nas/", &cwd).1, SLOW);
    assert_eq!(resolve("~/a/../Music", &cwd), "/home/demo/Music");
}
