use super::*;

fn run(script: &str) -> Result<Config, String> {
    execute(script, "test.lua")
}

#[test]
fn defaults_and_partial_updates_preserve_prior_values() {
    assert_eq!(run("").unwrap(), Config::default());
    let config = run(r##"
        local m = require('mmv')
        m.setup({view={position='left', max_width=800}, colorscheme='light'})
        m.setup({view={max_width=750.5}})
        m.set_hl('Normal', {fg='#aBcDeF', bg='#010203'})
        m.set_hl('Normal', {bg='#112233'})
    "##)
    .unwrap();
    assert_eq!(config.position, Position::Left);
    assert_eq!(config.max_width, 750.5);
    assert_eq!(config.colorscheme, ColorScheme::Light);
    assert_eq!(
        run("require('mmv').setup({view={max_width=1e100}})")
            .unwrap()
            .max_width,
        1e100
    );
    assert_eq!(
        config.highlights[&Group::Normal],
        Highlight {
            fg: Some(Color(0xab, 0xcd, 0xef)),
            bg: Some(Color(0x11, 0x22, 0x33))
        }
    );
}

#[test]
fn all_documented_groups_are_accepted() {
    let groups = [
        "Normal",
        "Border",
        "Search",
        "SearchCurrent",
        "StatusLine",
        "CommandLine",
        "ModeNormal",
        "ModeSearch",
        "ModeCommand",
        "Heading1",
        "Heading2",
        "Heading3",
        "Heading4",
        "Heading5",
        "Heading6",
        "Link",
        "CodeBlock",
        "Quote",
        "Panel",
        "PanelSelected",
        "Error",
        "Warning",
    ];
    let mut script = String::from("local m=require('mmv')\n");
    for group in groups {
        let attributes = match group {
            "Search" | "SearchCurrent" => "bg='#123456'",
            "Border" => "fg='#123456'",
            _ => "fg='#123456',bg='#654321'",
        };
        script.push_str(&format!("m.set_hl('{group}', {{{attributes}}})\n"));
    }
    assert_eq!(run(&script).unwrap().highlights.len(), groups.len());
}

#[test]
fn invalid_values_reject_the_entire_script_even_if_caught() {
    let invalid = [
        "m.setup({unknown=1})",
        "m.setup({[1]=1})",
        "m.setup({view={unknown=1}})",
        "m.setup({view='left'})",
        "m.setup({view={position='right'}})",
        "m.setup({view={position=42}})",
        "m.setup({colorscheme='blue'})",
        "m.setup({view={max_width='42'}})",
        "m.setup({view={max_width=0}})",
        "m.setup({view={max_width=-1}})",
        "m.setup({view={max_width=0/0}})",
        "m.setup({view={max_width=1/0}})",
        "m.set_hl('NoSuch',{})",
        "m.set_hl('Normal',{bold=true})",
        "m.set_hl('Search',{fg='#123456'})",
        "m.set_hl('SearchCurrent',{fg='#123456'})",
        "m.set_hl('Border',{bg='#123456'})",
        "m.set_hl('Normal',{[1]='x'})",
        "m.set_hl('Normal',{fg=123})",
        "m.set_hl('Normal',{fg='#123'})",
        "m.set_hl('Normal',{fg='#12zz56'})",
        "m.setup()",
        "m.set_hl('Normal')",
    ];
    for input in invalid {
        let script = format!(
            "local m=require('mmv'); m.setup({{view={{position='left'}}}}); m.set_hl('Normal',{{fg='#abcdef'}}); pcall(function() {input} end)"
        );
        assert!(run(&script).is_err(), "accepted {input}");
    }
    assert!(run("local m=require('mmv'); m.setup({colorscheme='light'}); error('oops')").is_err());
    assert!(run("not lua syntax !").is_err());
}
