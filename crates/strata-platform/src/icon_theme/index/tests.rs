use super::{DirKind, DirParams, parse};

#[test]
fn both_directory_lists_are_read_with_the_spec_defaults() {
    let text = "\
# A comment
[Icon Theme]
Name=T
Name[de]=T de
Inherits = Parent, ,hicolor,
Directories=48x48/apps,scalable/apps,48x48/apps,missing/apps
ScaledDirectories=48x48@2/apps

[48x48/apps]
Size=48

[scalable/apps]
Size=64
MinSize=8
MaxSize=512
Type=Scalable

[48x48@2/apps]
Size=48
Scale=2
Type=Fixed
Threshold=9
";
    let index = parse(text).expect("an index");
    assert_eq!(index.inherits, ["Parent", "hicolor"]);
    let dirs: Vec<(&str, DirParams)> = index
        .dirs
        .iter()
        .map(|d| (d.path.as_str(), d.params))
        .collect();
    let params = |kind, size, min, max, threshold, scale| DirParams {
        kind,
        size,
        min,
        max,
        threshold,
        scale,
    };
    assert_eq!(
        dirs,
        [
            ("48x48/apps", params(DirKind::Threshold, 48, 48, 48, 2, 1)),
            ("scalable/apps", params(DirKind::Scalable, 64, 8, 512, 2, 1)),
            ("48x48@2/apps", params(DirKind::Fixed, 48, 48, 48, 9, 2)),
        ]
    );
}

#[test]
fn a_directory_without_a_whole_size_is_left_out() {
    let text = "[Icon Theme]\nDirectories=a,b,c\n[a]\nSize=big\n[b]\nType=Fixed\n[c]\nSize=16\nType=Odd\nMinSize=x\n";
    let index = parse(text).expect("an index");
    assert_eq!(index.dirs.len(), 1);
    let c = index.dirs[0].params;
    // An unknown Type is Threshold, as in GTK; a bad MinSize takes the default (GTK reads 0).
    assert_eq!((c.kind, c.min), (DirKind::Threshold, 16));
}

#[test]
fn a_directory_outside_the_theme_is_left_out() {
    let text = "[Icon Theme]\nDirectories=/usr/share/pixmaps,../Other/48x48,a/../../b,./in,in/\nScaledDirectories=/abs\n[/usr/share/pixmaps]\nSize=48\n[../Other/48x48]\nSize=48\n[a/../../b]\nSize=48\n[./in]\nSize=48\n[in/]\nSize=48\n[/abs]\nSize=48\n";
    let index = parse(text).expect("an index");
    let paths: Vec<&str> = index.dirs.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["./in", "in/"]);
}

#[test]
fn no_index_without_the_head_group_or_its_directories() {
    assert_eq!(parse("[Other]\nDirectories=a\n[a]\nSize=1\n"), None);
    assert_eq!(parse("[Icon Theme]\nName=T\n"), None);
}
