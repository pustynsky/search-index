//! TypeScript parser tests — split from definitions_tests.rs.

use super::*;
use super::parser_typescript::parse_typescript_definitions_with_components;
use super::parser_csharp::parse_csharp_definitions;  // needed for test_ts_csharp_callers_still_work
use std::path::PathBuf;

fn parse_typescript_for_test(
    parser: &mut tree_sitter::Parser,
    source: &str,
    file_id: u32,
) -> ParseResult {
    parse_typescript_definitions_with_components(parser, source, file_id).0
}

// ─── TypeScript Parsing Tests ────────────────────────────────────────

#[test]
fn test_parse_ts_class() {
    let source = "export class UserService extends BaseService implements IUserService { }";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "UserService");
    assert!(class_defs[0].base_types.iter().any(|b| b.contains("BaseService")));
    assert!(class_defs[0].base_types.iter().any(|b| b.contains("IUserService")));
    assert!(class_defs[0].modifiers.contains(&"export".to_string()));
}

#[test]
fn test_parse_ts_abstract_class() {
    let source = r#"abstract class AbstractHandler {
    abstract handle(): void;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AbstractHandler");
    assert!(class_defs[0].modifiers.contains(&"abstract".to_string()));

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert!(!method_defs.is_empty());
    assert_eq!(method_defs[0].name, "handle");
    assert!(method_defs[0].modifiers.contains(&"abstract".to_string()));
}

#[test]
fn test_parse_ts_interface() {
    let source = r#"export interface IOrderProcessor {
    process(order: Order): Promise<void>;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let iface_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Interface).collect();
    assert_eq!(iface_defs.len(), 1);
    assert_eq!(iface_defs[0].name, "IOrderProcessor");
    assert!(iface_defs[0].modifiers.contains(&"export".to_string()));

    // Interface should have a property child for the method signature
    let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
    assert!(!prop_defs.is_empty());
}

#[test]
fn test_parse_ts_function() {
    let source = "export async function fetchUser(id: string): Promise<User> { return {} as User; }";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let fn_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Function).collect();
    assert_eq!(fn_defs.len(), 1);
    assert_eq!(fn_defs[0].name, "fetchUser");
    assert!(fn_defs[0].modifiers.contains(&"export".to_string()));
    assert!(fn_defs[0].modifiers.contains(&"async".to_string()));
    assert!(fn_defs[0].signature.is_some());
    let sig = fn_defs[0].signature.as_ref().unwrap();
    assert!(sig.contains("id: string"));
}

#[test]
fn test_parse_ts_method() {
    let source = r#"class UserManager {
    public async getUser(id: string): Promise<User> { return {} as User; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert_eq!(method_defs.len(), 1);
    assert_eq!(method_defs[0].name, "getUser");
    assert!(method_defs[0].modifiers.contains(&"public".to_string()));
    assert!(method_defs[0].modifiers.contains(&"async".to_string()));
    assert_eq!(method_defs[0].parent, Some("UserManager".to_string()));
}

#[test]
fn test_parse_ts_constructor() {
    let source = r#"class OrderService {
    constructor(private userService: IUserService) { }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ctor_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Constructor).collect();
    assert_eq!(ctor_defs.len(), 1);
    assert_eq!(ctor_defs[0].name, "constructor");
    assert_eq!(ctor_defs[0].parent, Some("OrderService".to_string()));
    assert!(ctor_defs[0].signature.is_some());
    let sig = ctor_defs[0].signature.as_ref().unwrap();
    assert!(sig.contains("userService"));
}

#[test]
fn test_parse_ts_enum() {
    let source = r#"export enum OrderStatus {
    Pending,
    Active,
    Completed
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "OrderStatus");

    let member_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(member_defs.len(), 3);
}

#[test]
fn test_parse_ts_const_enum() {
    let source = r#"const enum Foo {
    Alpha,
    Beta,
    Gamma
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "Foo");

    let member_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(member_defs.len(), 3);
    let member_names: Vec<&str> = member_defs.iter().map(|d| d.name.as_str()).collect();
    assert!(member_names.contains(&"Alpha"));
    assert!(member_names.contains(&"Beta"));
    assert!(member_names.contains(&"Gamma"));
    for m in &member_defs {
        assert_eq!(m.parent.as_deref(), Some("Foo"));
    }
}

#[test]
fn test_parse_ts_type_alias() {
    let source = "export type UserId = string | number;";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ta_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::TypeAlias).collect();
    assert_eq!(ta_defs.len(), 1);
    assert_eq!(ta_defs[0].name, "UserId");
    assert!(ta_defs[0].modifiers.contains(&"export".to_string()));
}

#[test]
fn test_parse_ts_variable() {
    let source = "export const MAX_RETRIES = 3;";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let var_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Variable).collect();
    assert_eq!(var_defs.len(), 1);
    assert_eq!(var_defs[0].name, "MAX_RETRIES");
    assert!(var_defs[0].modifiers.contains(&"export".to_string()));

    assert!(!var_defs[0].modifiers.contains(&"callable".to_string()));
}

#[test]
fn test_parse_ts_decorators() {
    let source = r#"@Injectable()
@Component({selector: 'app'})
class AppComponent {}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AppComponent");
    assert_eq!(class_defs[0].attributes.len(), 2);
    assert!(class_defs[0].attributes.iter().any(|a| a.contains("Injectable")));
    assert!(class_defs[0].attributes.iter().any(|a| a.contains("Component")));
}

#[test]
fn test_parse_ts_field() {
    let source = r#"class DataHolder {
    private readonly name: string = '';
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let field_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Field).collect();
    assert_eq!(field_defs.len(), 1);
    assert_eq!(field_defs[0].name, "name");
    assert!(field_defs[0].modifiers.contains(&"private".to_string()));
    assert!(field_defs[0].modifiers.contains(&"readonly".to_string()));
}

#[test]
fn test_parse_ts_interface_property() {
    let source = r#"interface IEntity {
    readonly id: string;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
    assert_eq!(prop_defs.len(), 1);
    assert_eq!(prop_defs[0].name, "id");
    assert!(prop_defs[0].modifiers.contains(&"readonly".to_string()));
}

#[test]
fn test_parse_tsx_file() {
    let source = r#"export class AppComponent {
    render() { return <div/>; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AppComponent");

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert_eq!(method_defs.len(), 1);
    assert_eq!(method_defs[0].name, "render");
}

#[test]
fn test_ts_incremental_update() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Step 1: Create a .ts file and add it to the index
    let test_file = dir.join("service.ts");
    std::fs::write(&test_file, "export class OrderService { process(): void {} }").unwrap();

    let mut index = DefinitionIndex {
        root: ".".to_string(), extensions: vec!["ts".to_string()],
        ..Default::default()
    };

    let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));
    update_file_definitions(&mut index, &clean);

    assert!(!index.definitions.is_empty());
    assert!(index.name_index.contains_key("orderservice"));
    assert!(index.name_index.contains_key("process"));
    assert_eq!(index.files.len(), 1);

    // Step 2: Modify the .ts file — rename class, add a method
    std::fs::write(&test_file, r#"export class UpdatedService {
    execute(): void {}
    validate(): boolean { return true; }
}"#).unwrap();

    update_file_definitions(&mut index, &clean);

    assert!(!index.name_index.contains_key("orderservice"));
    assert!(!index.name_index.contains_key("process"));
    assert!(index.name_index.contains_key("updatedservice"));
    assert!(index.name_index.contains_key("execute"));
    assert!(index.name_index.contains_key("validate"));

    // Step 3: Remove the file (simulate deletion by writing empty)
    std::fs::write(&test_file, "").unwrap();
    update_file_definitions(&mut index, &clean);

    // All named definitions from that file should be gone from name index
    assert!(!index.name_index.contains_key("updatedservice"));
    assert!(!index.name_index.contains_key("execute"));
    assert!(!index.name_index.contains_key("validate"));
}


// ─── TypeScript Call-Site Extraction Tests ────────────────────────────

#[test]
fn test_ts_this_method_call() {
    let source = r#"class OrderService {
    process(): void {
        this.doSomething();
    }
    doSomething(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process' method");
    let ds = pc[0].1.iter().find(|c| c.method_name == "doSomething");
    assert!(ds.is_some(), "Expected call to 'doSomething'");
    assert_eq!(ds.unwrap().receiver_type.as_deref(), Some("OrderService"));
}

#[test]
fn test_ts_this_field_method_call() {
    let source = r#"class OrderController {
    constructor(private userService: UserService) {}
    handle(): void {
        this.userService.getUser();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let hi = defs.iter().position(|d| d.name == "handle").unwrap();
    let hc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == hi).collect();
    assert!(!hc.is_empty(), "Expected call sites for 'handle' method");
    let gu = hc[0].1.iter().find(|c| c.method_name == "getUser");
    assert!(gu.is_some(), "Expected call to 'getUser'");
    assert_eq!(gu.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_standalone_function_call() {
    let source = r#"function processOrder(): void {
    someHelper();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "processOrder").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'processOrder'");
    let sh = pc[0].1.iter().find(|c| c.method_name == "someHelper");
    assert!(sh.is_some(), "Expected call to 'someHelper'");
    assert_eq!(sh.unwrap().receiver_type, None);
}

#[test]
fn test_ts_bare_call_lexical_bindings_are_typed() {
    let source = r#"import defaultFn from './default';
import * as namespaceFn from './namespace';
import { remote as importedFn } from './remote';
function sameModule(): void {}
function recursive(): void { recursive(); }
export function scenario(param: () => void): void {
    const local = () => {};
    local();
    param();
    let reassigned = () => {};
    reassigned = sameModule;
    reassigned();
    let changing = () => {};
    changing();
    changing = sameModule;
    changing();
    try {} catch (error) { error(); }
    defaultFn();
    namespaceFn();
    namespaceFn.remote();
    importedFn();
    console.log('x');
    setTimeout(() => {}, 0);
    sameModule();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let scenario_idx = defs.iter().position(|definition| definition.name == "scenario").unwrap();
    let scenario_calls = &call_sites
        .iter()
        .find(|(definition, _)| *definition == scenario_idx)
        .unwrap()
        .1;
    let kind = |name: &str| {
        scenario_calls
            .iter()
            .find(|call| call.method_name == name)
            .map(|call| call.call_kind)
            .unwrap()
    };
    assert!(matches!(
        kind("local"),
        CallSiteKind::TypeScriptLocalCallable { .. }
    ));
    assert_eq!(
        kind("param"),
        CallSiteKind::TypeScriptDynamicCallableParameter
    );
    assert_eq!(
        kind("reassigned"),
        CallSiteKind::TypeScriptUnresolvedLocal
    );
    assert_eq!(kind("importedFn"), CallSiteKind::TypeScriptImported);

    assert_eq!(kind("defaultFn"), CallSiteKind::TypeScriptImported);
    assert_eq!(kind("namespaceFn"), CallSiteKind::TypeScriptImported);

    assert_eq!(kind("remote"), CallSiteKind::TypeScriptImported);
    assert_eq!(kind("error"), CallSiteKind::TypeScriptUnresolvedLocal);
    let changing_kinds: Vec<_> = scenario_calls
        .iter()
        .filter(|call| call.method_name == "changing")
        .map(|call| call.call_kind)
        .collect();
    assert!(matches!(
        changing_kinds.as_slice(),
        [CallSiteKind::TypeScriptLocalCallable { .. }, CallSiteKind::TypeScriptUnresolvedLocal]
    ));
    assert_eq!(kind("setTimeout"), CallSiteKind::TypeScriptUnknownGlobal);

    assert_eq!(kind("log"), CallSiteKind::TypeScriptUnknownGlobal);
    assert!(matches!(
        kind("sameModule"),
        CallSiteKind::TypeScriptSameFile { .. }
    ));

    let recursive_idx = defs
        .iter()
        .position(|definition| definition.name == "recursive" && definition.parent.is_none())
        .unwrap();
    let recursive_call = call_sites
        .iter()
        .find(|(definition, _)| *definition == recursive_idx)
        .unwrap()
        .1
        .iter()
        .find(|call| call.method_name == "recursive")
        .unwrap();
    assert!(matches!(
        recursive_call.call_kind,
        CallSiteKind::TypeScriptSameFile { .. }
    ));
}

#[test]
fn test_ts_var_callable_uses_nearest_function_scope() {
    let source = r#"function moduleFn(): void {}
export function outer(flag: boolean): void {
    if (flag) {
        var moduleFn = () => {};
    }
    moduleFn();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);
    let outer_idx = defs
        .iter()
        .position(|definition| definition.name == "outer")
        .unwrap();
    let call = call_sites
        .iter()
        .find(|(definition, _)| *definition == outer_idx)
        .unwrap()
        .1
        .iter()
        .find(|call| call.method_name == "moduleFn")
        .unwrap();
    assert!(matches!(
        call.call_kind,
        CallSiteKind::TypeScriptLocalCallable { .. }
    ));
}


#[test]
fn test_ts_bare_call_scopes_preserve_shadowing_and_closure_capture() {
    let source = r#"export function outer(flag: boolean): void {
    function nested(): void {}
    const captured = () => {};
    if (flag) {
        const branch = () => {};
        branch();
    } else {
        const branch = function(): void {};
        branch();
    }
    function inner(): void { captured(); }
    nested();
    inner();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let outer_idx = defs
        .iter()
        .position(|definition| definition.name == "outer" && definition.parent.is_none())
        .unwrap();
    let outer_calls = &call_sites
        .iter()
        .find(|(definition, _)| *definition == outer_idx)
        .unwrap()
        .1;
    for name in ["nested", "inner"] {
        assert!(matches!(
            outer_calls
                .iter()
                .find(|call| call.method_name == name)
                .unwrap()
                .call_kind,
            CallSiteKind::TypeScriptLocalCallable { .. }
        ));
    }

    let branch_target_lines: std::collections::HashSet<u32> = defs
        .iter()
        .filter(|definition| definition.name == "branch")
        .map(|definition| definition.line_start)
        .collect();
    let branch_call_lines: std::collections::HashSet<u32> = outer_calls
        .iter()
        .filter_map(|call| match call.call_kind {
            CallSiteKind::TypeScriptLocalCallable { definition_line }
                if call.method_name == "branch" => Some(definition_line),
            _ => None,
        })
        .collect();
    assert_eq!(branch_call_lines, branch_target_lines);

    let inner_idx = defs
        .iter()
        .position(|definition| definition.name == "inner")
        .unwrap();
    let captured_call = call_sites
        .iter()
        .find(|(definition, _)| *definition == inner_idx)
        .unwrap()
        .1
        .iter()
        .find(|call| call.method_name == "captured")
        .unwrap();
    assert!(matches!(
        captured_call.call_kind,
        CallSiteKind::TypeScriptLocalCallable { .. }
    ));
}


#[test]
fn test_ts_destructured_bindings_and_loop_reassignments_are_conservative() {
    let source = r#"function target(): void {}
function render(): void {}
function inspect(): void {}
export function scenario(
    { callback }: { callback: () => void },
    [arrayCallback]: Array<() => void>,
    renderers: Array<() => void>,
    registry: Record<string, () => void>
): void {
    const { local } = source;
    let exact = () => {};
    [exact] = source;
    let deferred = () => {};
    on(() => { deferred(); });
    deferred = target;
    let looped = () => {};
    while (condition) {
        looped();
        looped = target;
    }
    for (const render of renderers) {
        render();
    }
    for (let inspect in registry) {
        inspect();
    }
    render();
    inspect();
    callback();
    arrayCallback();
    local();
    exact();
    let postfix = () => {};
    postfix++;
    postfix();
    let prefix = () => {};
    ++prefix;
    prefix();
    if (condition) {
        var duplicate = () => {};
    } else {
        var duplicate = () => {};
    }
    duplicate();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let scenario_idx = defs.iter().position(|definition| definition.name == "scenario").unwrap();
    let calls = &call_sites
        .iter()
        .find(|(definition, _)| *definition == scenario_idx)
        .unwrap()
        .1;
    let kind = |name: &str| {
        calls
            .iter()
            .find(|call| call.method_name == name)
            .map(|call| call.call_kind)
            .unwrap()
    };
    assert_eq!(
        kind("callback"),
        CallSiteKind::TypeScriptDynamicCallableParameter
    );
    assert_eq!(
        kind("arrayCallback"),
        CallSiteKind::TypeScriptDynamicCallableParameter
    );
    for name in ["render", "inspect"] {
        let kinds: Vec<_> = calls
            .iter()
            .filter(|call| call.method_name == name)
            .map(|call| call.call_kind)
            .collect();
        assert!(matches!(
            kinds.as_slice(),
            [CallSiteKind::TypeScriptUnresolvedLocal, CallSiteKind::TypeScriptSameFile { .. }]
        ), "{name}: {kinds:?}");
    }


    for name in [
        "local",
        "exact",
        "deferred",
        "looped",
        "postfix",
        "prefix",
        "duplicate",
    ] {
        assert_eq!(
            kind(name),
            CallSiteKind::TypeScriptUnresolvedLocal,
            "{name} must not create an exact edge"
        );
    }
}


#[test]
fn test_ts_same_named_nested_callables_keep_independent_call_sites() {
    let source = r#"function outerTarget(): void {}
function innerTarget(): void {}
function visit(): void {}
export function run(): void {
    outerTarget();
    const run = () => { innerTarget(); };
    run();
}
export const walk = (node: Node): void => {
    const walk = (child: Node, depth: number): void => { visit(); };
    walk(node, 0);
};"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let call_site_owners: std::collections::HashSet<_> =
        call_sites.iter().map(|(definition, _)| *definition).collect();
    assert_eq!(call_site_owners.len(), call_sites.len(), "{call_sites:#?}");

    let assert_independent =
        |name: &str, outer_call: &str, inner_call: &str, inner_signature: &str| {
        let matching: Vec<_> = defs
            .iter()
            .enumerate()
            .filter(|(_, definition)| definition.name == name)
            .collect();
        assert_eq!(matching.len(), 2, "{name}: {defs:#?}");
        let (outer_index, _) = matching
            .iter()
            .find(|(_, definition)| {
                !definition.modifiers.iter().any(|modifier| modifier == "local")
            })
            .copied()
            .unwrap();
        let (inner_index, inner_definition) = matching
            .iter()
            .find(|(_, definition)| {
                definition.modifiers.iter().any(|modifier| modifier == "local")
            })
            .copied()
            .unwrap();
        assert_eq!(
            inner_definition.signature.as_deref(),
            Some(inner_signature),
            "{inner_definition:#?}"
        );
        let outer_calls = &call_sites
            .iter()
            .find(|(definition, _)| *definition == outer_index)
            .unwrap()
            .1;
        let inner_calls = &call_sites
            .iter()
            .find(|(definition, _)| *definition == inner_index)
            .unwrap()
            .1;
        assert!(outer_calls.iter().any(|call| call.method_name == outer_call));
        assert!(!outer_calls.iter().any(|call| call.method_name == inner_call));
        assert!(inner_calls.iter().any(|call| call.method_name == inner_call));
        let recursive_call = outer_calls
            .iter()
            .find(|call| call.method_name == name)
            .unwrap();
        assert_eq!(
            recursive_call.call_kind,
            CallSiteKind::TypeScriptLocalCallable {
                definition_line: inner_definition.line_start,
            }
        );
    };

    assert_independent("run", "outerTarget", "innerTarget", "run()");
    assert_independent(
        "walk",
        "walk",
        "visit",
        "walk(child: Node, depth: number): void",
    );
}


#[test]
fn test_ts_synthetic_callables_preserve_class_context_and_avoid_duplicates() {
    let source = r#"function topLevel(): void {}
export const
    firstArrow = () => {},
    exportedArrow = () => { topLevel(); };
const moduleArrow = () => { topLevel(); };
export function callExportedArrow(): void { exportedArrow(); }
if (flag) { function helper(): void {} helper(); }
class OrderService {
    constructor(private repo: OrderRepo) {}
    log(): void {}
    process(): void {
        const doIt = () => {
            this.repo.save();
            this.log();
        };
        doIt();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    for name in ["firstArrow", "exportedArrow", "moduleArrow", "helper", "doIt"] {
        assert_eq!(
            defs.iter().filter(|definition| definition.name == name).count(),
            1,
            "duplicate definition for {name}: {defs:#?}"
        );
    }

    let exported_idx = defs
        .iter()
        .position(|definition| definition.name == "exportedArrow")
        .unwrap();
    assert_eq!(defs[exported_idx].kind, DefinitionKind::Variable);

    assert!(
        defs[exported_idx]
            .modifiers
            .iter()
            .any(|modifier| modifier == "callable")
    );
    assert!(call_sites.iter().any(|(definition, calls)| {
        *definition == exported_idx && calls.iter().any(|call| call.method_name == "topLevel")
    }));

    let caller_idx = defs
        .iter()
        .position(|definition| definition.name == "callExportedArrow")
        .unwrap();
    let exported_call = call_sites
        .iter()
        .find(|(definition, _)| *definition == caller_idx)
        .unwrap()
        .1
        .iter()
        .find(|call| call.method_name == "exportedArrow")
        .unwrap();
    assert_eq!(
        exported_call.call_kind,
        CallSiteKind::TypeScriptSameFile {
            definition_line: defs[exported_idx].line_start,
        }
    );

    let module_arrow = defs
        .iter()
        .find(|definition| definition.name == "moduleArrow")
        .unwrap();
    assert!(module_arrow.modifiers.iter().any(|modifier| modifier == "local"));

    let do_it_idx = defs.iter().position(|definition| definition.name == "doIt").unwrap();
    assert_eq!(defs[do_it_idx].parent.as_deref(), Some("OrderService"));
    assert!(defs[do_it_idx].modifiers.iter().any(|modifier| modifier == "local"));
    let do_it_calls = &call_sites
        .iter()
        .find(|(definition, _)| *definition == do_it_idx)
        .unwrap()
        .1;
    assert_eq!(
        do_it_calls
            .iter()
            .find(|call| call.method_name == "save")
            .and_then(|call| call.receiver_type.as_deref()),
        Some("OrderRepo")
    );
    assert_eq!(
        do_it_calls
            .iter()
            .find(|call| call.method_name == "log")
            .and_then(|call| call.receiver_type.as_deref()),
        Some("OrderService")
    );
}


#[test]
fn test_ts_lexical_walkers_stop_at_the_typescript_depth_limit() {
    const DEPTH: usize = 300;
    let mut source = String::from(
        "function target(): void {}\nexport function deep(): void { target();",
    );
    for _ in 0..DEPTH {
        source.push('{');
    }
    source.push_str("const deepest = () => {}; deepest();");
    for _ in 0..DEPTH {
        source.push('}');
    }
    source.push('}');

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, &source, 0);

    assert!(defs.iter().any(|definition| definition.name == "deep"));
    assert!(!defs.iter().any(|definition| definition.name == "deepest"));
    let deep_index = defs
        .iter()
        .position(|definition| definition.name == "deep")
        .unwrap();
    let deep_calls = &call_sites
        .iter()
        .find(|(definition, _)| *definition == deep_index)
        .unwrap()
        .1;
    assert!(!deep_calls.iter().any(|call| call.method_name == "deepest"));
    assert_eq!(
        deep_calls
            .iter()
            .find(|call| call.method_name == "target")
            .unwrap()
            .call_kind,
        CallSiteKind::TypeScriptAnalysisIncomplete
    );
    assert!(deep_calls.iter().any(|call| {
        call.call_kind == CallSiteKind::TypeScriptAnalysisIncomplete
            && call.method_name == "<ast-depth-limit>"
    }));
}


#[test]
fn test_ts_new_expression() {
    let source = r#"class Factory {
    create(): void {
        const svc = new UserService();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ci = defs.iter().position(|d| d.name == "create").unwrap();
    let cc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ci).collect();
    assert!(!cc.is_empty(), "Expected call sites for 'create'");
    let nc = cc[0].1.iter().find(|c| c.method_name == "UserService");
    assert!(nc.is_some(), "Expected new UserService call");
    assert_eq!(nc.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_static_method_call() {
    let source = r#"class Processor {
    run(): void {
        MathUtils.calculate();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run'");
    let mc = rc[0].1.iter().find(|c| c.method_name == "calculate");
    assert!(mc.is_some(), "Expected call to 'calculate'");
    assert_eq!(mc.unwrap().receiver_type.as_deref(), Some("MathUtils"));
}

#[test]
fn test_ts_arrow_function_class_property() {
    let source = r#"class ItemProcessor {
    processItem = (item: string): void => {
        this.validate(item);
    };
    validate(item: string): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let process_item_defs: Vec<_> = defs
        .iter()
        .filter(|definition| definition.name == "processItem")
        .collect();
    assert_eq!(process_item_defs.len(), 1, "{process_item_defs:#?}");
    assert_eq!(process_item_defs[0].kind, DefinitionKind::Field);

    assert!(
        process_item_defs[0]
            .modifiers
            .iter()
            .any(|modifier| modifier == "callable")
    );


    let pi = defs.iter().position(|d| d.name == "processItem").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'processItem' arrow function");
    let vc = pc[0].1.iter().find(|c| c.method_name == "validate");
    assert!(vc.is_some(), "Expected call to 'validate'");
    assert_eq!(vc.unwrap().receiver_type.as_deref(), Some("ItemProcessor"));
}

#[test]
fn test_ts_constructor_di_field_types() {
    let source = r#"class OrderHandler {
    constructor(private orderRepo: OrderRepository, private logger: Logger) {}
    execute(): void {
        this.orderRepo.save();
        this.logger.info("done");
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ei = defs.iter().position(|d| d.name == "execute").unwrap();
    let ec: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ei).collect();
    assert!(!ec.is_empty(), "Expected call sites for 'execute'");

    let save = ec[0].1.iter().find(|c| c.method_name == "save");
    assert!(save.is_some(), "Expected call to 'save'");
    assert_eq!(save.unwrap().receiver_type.as_deref(), Some("OrderRepository"));

    let info = ec[0].1.iter().find(|c| c.method_name == "info");
    assert!(info.is_some(), "Expected call to 'info'");
    assert_eq!(info.unwrap().receiver_type.as_deref(), Some("Logger"));
}

#[test]
fn test_ts_multiple_calls_in_method() {
    let source = r#"class DataService {
    constructor(private repo: DataRepository) {}
    process(): void {
        this.validate();
        this.repo.findAll();
        const result = new ResultSet();
        helperFn();
        Formatter.format();
    }
    validate(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process'");

    let names: Vec<&str> = pc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"validate"), "Expected call to 'validate'");
    assert!(names.contains(&"findAll"), "Expected call to 'findAll'");
    assert!(names.contains(&"ResultSet"), "Expected new ResultSet");
    assert!(names.contains(&"helperFn"), "Expected call to 'helperFn'");
    assert!(names.contains(&"format"), "Expected call to 'format'");

    // Check receiver types
    let validate_call = pc[0].1.iter().find(|c| c.method_name == "validate").unwrap();
    assert_eq!(validate_call.receiver_type.as_deref(), Some("DataService"));
    let find_call = pc[0].1.iter().find(|c| c.method_name == "findAll").unwrap();
    assert_eq!(find_call.receiver_type.as_deref(), Some("DataRepository"));
    let helper_call = pc[0].1.iter().find(|c| c.method_name == "helperFn").unwrap();
    assert_eq!(helper_call.receiver_type, None);
    let fmt_call = pc[0].1.iter().find(|c| c.method_name == "format").unwrap();
    assert_eq!(fmt_call.receiver_type.as_deref(), Some("Formatter"));
}

#[test]
fn test_ts_no_calls_empty_body() {
    let source = r#"class EmptyService {
    doNothing(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "doNothing").unwrap();
    let nc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ni).collect();
    assert!(nc.is_empty(), "Expected no call sites for empty method");
}

#[test]
fn test_ts_class_field_type() {
    let source = r#"class CachedService {
    private cache: CacheService;
    lookup(): void {
        this.cache.get();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let li = defs.iter().position(|d| d.name == "lookup").unwrap();
    let lc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == li).collect();
    assert!(!lc.is_empty(), "Expected call sites for 'lookup'");
    let gc = lc[0].1.iter().find(|c| c.method_name == "get");
    assert!(gc.is_some(), "Expected call to 'get'");
    assert_eq!(gc.unwrap().receiver_type.as_deref(), Some("CacheService"));
}

#[test]
fn test_ts_csharp_callers_still_work() {
    let source = r#"
public class NotificationService {
    private readonly IEmailSender _sender;
    public NotificationService(IEmailSender sender) { _sender = sender; }
    public void Notify(string message) { _sender.Send(message); this.LogResult(); }
    private void LogResult() {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs, _, _) = parse_csharp_definitions(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "Notify").unwrap();
    let nc: Vec<_> = cs.iter().filter(|(i, _)| *i == ni).collect();
    assert!(!nc.is_empty(), "Expected call sites for 'Notify' (C# regression)");

    let send = nc[0].1.iter().find(|c| c.method_name == "Send");
    assert!(send.is_some(), "Expected call to 'Send'");
    assert_eq!(send.unwrap().receiver_type.as_deref(), Some("IEmailSender"));

    let log = nc[0].1.iter().find(|c| c.method_name == "LogResult");
    assert!(log.is_some(), "Expected call to 'LogResult'");
    assert_eq!(log.unwrap().receiver_type.as_deref(), Some("NotificationService"));
}


#[test]
fn test_ts_inject_field_initializer() {
    let source = r#"class MyComponent {
    private readonly zone = inject(NgZone);
    private readonly userService = inject(UserService);
    run(): void {
        this.zone.run();
        this.userService.getUser();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run' method");

    let zone_call = rc[0].1.iter().find(|c| c.method_name == "run" && c.receiver_type.is_some());
    assert!(zone_call.is_some(), "Expected call to 'zone.run()'");
    assert_eq!(zone_call.unwrap().receiver_type.as_deref(), Some("NgZone"));

    let user_call = rc[0].1.iter().find(|c| c.method_name == "getUser");
    assert!(user_call.is_some(), "Expected call to 'userService.getUser()'");
    assert_eq!(user_call.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_inject_constructor_assignment() {
    let source = r#"class MyComponent {
    constructor() {
        this.store = inject(Store);
        this.router = inject(Router);
    }
    navigate(): void {
        this.store.dispatch();
        this.router.navigate();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "navigate").unwrap();
    let nc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ni).collect();
    assert!(!nc.is_empty(), "Expected call sites for 'navigate' method");

    let store_call = nc[0].1.iter().find(|c| c.method_name == "dispatch");
    assert!(store_call.is_some(), "Expected call to 'store.dispatch()'");
    assert_eq!(store_call.unwrap().receiver_type.as_deref(), Some("Store"));

    let router_call = nc[0].1.iter().find(|c| c.method_name == "navigate" && c.receiver_type.is_some());
    assert!(router_call.is_some(), "Expected call to 'router.navigate()'");
    assert_eq!(router_call.unwrap().receiver_type.as_deref(), Some("Router"));
}

#[test]
fn test_ts_inject_with_generic() {
    let source = r#"class MyComponent {
    private store = inject(Store<AppState>);
    doWork(): void {
        this.store.dispatch();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let di = defs.iter().position(|d| d.name == "doWork").unwrap();
    let dc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == di).collect();
    assert!(!dc.is_empty(), "Expected call sites for 'doWork' method");

    let store_call = dc[0].1.iter().find(|c| c.method_name == "dispatch");
    assert!(store_call.is_some(), "Expected call to 'store.dispatch()'");
    assert_eq!(store_call.unwrap().receiver_type.as_deref(), Some("Store"));
}


// ─── TypeScript Interface Resolution Tests ───────────────────────────

#[test]
fn test_ts_interface_implements_extracted() {
    let source = r#"
interface IUserService {
    getUser(): void;
}

class UserService implements IUserService {
    getUser(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "UserService");
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IUserService")),
        "Expected base_types to contain 'IUserService', got: {:?}",
        class_defs[0].base_types
    );
}

#[test]
fn test_ts_interface_call_through_field() {
    let source = r#"
interface IOrderService {
    processOrder(): void;
}

class OrderProcessor {
    constructor(private orderService: IOrderService) {}
    run(): void {
        this.orderService.processOrder();
    }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run' method");

    let po = rc[0].1.iter().find(|c| c.method_name == "processOrder");
    assert!(po.is_some(), "Expected call to 'processOrder'");
    assert_eq!(
        po.unwrap().receiver_type.as_deref(),
        Some("IOrderService"),
        "Expected receiver_type to be 'IOrderService'"
    );
}

#[test]
fn test_ts_multiple_implements() {
    let source = r#"
interface IReader {
    read(): void;
}
interface IWriter {
    write(): void;
}
class DataService implements IReader, IWriter {
    read(): void {}
    write(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class && d.name == "DataService").collect();
    assert_eq!(class_defs.len(), 1);
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IReader")),
        "Expected base_types to contain 'IReader', got: {:?}",
        class_defs[0].base_types
    );
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IWriter")),
        "Expected base_types to contain 'IWriter', got: {:?}",
        class_defs[0].base_types
    );
}

#[test]
fn test_ts_extends_and_implements() {
    let source = r#"
class BaseService {
    init(): void {}
}
interface IAdminService {
    manage(): void;
}
class AdminService extends BaseService implements IAdminService {
    manage(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class && d.name == "AdminService").collect();
    assert_eq!(class_defs.len(), 1);
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("BaseService")),
        "Expected base_types to contain 'BaseService', got: {:?}",
        class_defs[0].base_types
    );
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IAdminService")),
        "Expected base_types to contain 'IAdminService', got: {:?}",
        class_defs[0].base_types
    );
}

#[test]
fn test_parse_ts_injection_token_variable() {
    let source = "export const AUTH_TOKEN = new InjectionToken<IAuthService>('AUTH_TOKEN');";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let var_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Variable).collect();
    assert_eq!(var_defs.len(), 1, "Expected exactly one variable definition");
    assert_eq!(var_defs[0].name, "AUTH_TOKEN");
    assert!(var_defs[0].modifiers.contains(&"export".to_string()));
    assert!(var_defs[0].modifiers.contains(&"const".to_string()));

    // The parser currently captures type annotations but NOT initializer expressions.
    // For `const AUTH_TOKEN = new InjectionToken<IAuthService>(...)`, there is no explicit
    // type annotation, so the signature will be "const AUTH_TOKEN" without InjectionToken info.
    // TODO: To fully support InjectionToken patterns, the parser would need to extract
    // the initializer's constructor name (InjectionToken<IAuthService>) into the signature.
    let sig = var_defs[0].signature.as_ref().expect("Expected a signature");
    assert!(sig.contains("AUTH_TOKEN"), "Signature should contain the variable name");

    if sig.contains("InjectionToken") {
        // Parser captures initializer type — ideal behavior
        assert!(sig.contains("InjectionToken<IAuthService>"));
    } else {
        // Parser does NOT capture initializer — document the gap
        eprintln!(
            "NOTE: InjectionToken<IAuthService> NOT captured in signature. Signature: '{}'",
            sig
        );
    }
}

// ─── TypeScript Local Variable Type Extraction Tests ─────────────────

#[test]
fn test_ts_local_var_explicit_type_annotation() {
    let source = r#"class UserService {
    private repo: UserRepository;

    getUser(id: number): void {
        const result: UserResult = this.repo.findById(id);
        result.validate();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let gi = defs.iter().position(|d| d.name == "getUser").unwrap();
    let gc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == gi).collect();
    assert!(!gc.is_empty(), "Expected call sites for 'getUser'");

    let validate = gc[0].1.iter().find(|c| c.method_name == "validate");
    assert!(validate.is_some(), "Expected call to 'validate'");
    assert_eq!(
        validate.unwrap().receiver_type.as_deref(),
        Some("UserResult"),
        "Local var 'result' with explicit type annotation ':UserResult' should resolve receiver_type"
    );
}

#[test]
fn test_ts_local_var_new_expression() {
    let source = r#"class OrderService {
    processOrder(): void {
        const validator = new OrderValidator();
        validator.check();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "processOrder").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'processOrder'");

    let check = pc[0].1.iter().find(|c| c.method_name == "check");
    assert!(check.is_some(), "Expected call to 'check'");
    assert_eq!(
        check.unwrap().receiver_type.as_deref(),
        Some("OrderValidator"),
        "Local var 'validator' assigned from 'new OrderValidator()' should resolve receiver_type"
    );
}

#[test]
fn test_ts_inline_new_expression_receiver_type() {
    let source = r#"class XrayEdgeDerived {
    execute(value: string): string { return value; }
}
function xrayEdgeDirectCall(): string {
    return new XrayEdgeDerived().execute("marker");
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let function_index = defs
        .iter()
        .position(|definition| definition.name == "xrayEdgeDirectCall")
        .unwrap();
    let calls = call_sites
        .iter()
        .find(|(index, _)| *index == function_index)
        .unwrap();
    let execute = calls
        .1
        .iter()
        .find(|call| call.method_name == "execute")
        .unwrap();
    assert_eq!(execute.receiver_type.as_deref(), Some("XrayEdgeDerived"));
}

#[test]
fn test_ts_method_parameter_receiver_type() {
    let source = r#"namespace Models {
    export class GenericTarget<T> { execute(): void {} }
}
class ParameterTarget { execute(): void {} }
class WrongParameterTarget { execute(): void {} }
class ParameterCaller {
    private target: WrongParameterTarget;
    typed(target: ParameterTarget): void { target.execute(); }
    optional(target?: ParameterTarget): void { target?.execute(); }
    generic(target: Models.GenericTarget<string>): void { target.execute(); }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let (definitions, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    for (method_name, expected_receiver) in [
        ("typed", "ParameterTarget"),
        ("optional", "ParameterTarget"),
        ("generic", "GenericTarget"),
    ] {
        let method_index = definitions
            .iter()
            .position(|definition| definition.name == method_name)
            .unwrap();
        let calls = call_sites
            .iter()
            .find(|(index, _)| *index == method_index)
            .unwrap();
        let execute = calls
            .1
            .iter()
            .find(|call| call.method_name == "execute")
            .unwrap();
        assert_eq!(execute.receiver_type.as_deref(), Some(expected_receiver));
    }
}

#[test]
fn test_ts_conditional_receiver_requires_matching_branch_types() {
    let source = r#"class TargetA { execute(): void {} }
class TargetB { execute(): void {} }
class ConditionalCaller {
    sameTernary(condition: boolean, first: TargetA, second: TargetA): void {
        (condition ? first : second).execute();
    }
    differentTernary(condition: boolean, first: TargetA, second: TargetB): void {
        (condition ? first : second).execute();
    }
    sameCoalescing(first: TargetA | undefined, fallback: TargetA): void {
        (first ?? fallback).execute();
    }
    differentCoalescing(first: TargetA | undefined, fallback: TargetB): void {
        (first ?? fallback).execute();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let (definitions, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    for (method_name, expected_receiver) in [
        ("sameTernary", Some("TargetA")),
        ("differentTernary", None),
        ("sameCoalescing", Some("TargetA")),
        ("differentCoalescing", None),
    ] {
        let method_index = definitions
            .iter()
            .position(|definition| definition.name == method_name)
            .unwrap();
        let calls = call_sites
            .iter()
            .find(|(index, _)| *index == method_index)
            .unwrap();
        let execute = calls
            .1
            .iter()
            .find(|call| call.method_name == "execute")
            .unwrap();
        assert_eq!(execute.receiver_type.as_deref(), expected_receiver);
    }
}

#[test]
fn test_ts_local_var_new_expression_with_generics() {
    let source = r#"class DataService {
    loadData(): void {
        const cache = new DataCache<string>();
        cache.get("key");
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let li = defs.iter().position(|d| d.name == "loadData").unwrap();
    let lc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == li).collect();
    assert!(!lc.is_empty(), "Expected call sites for 'loadData'");

    let get = lc[0].1.iter().find(|c| c.method_name == "get");
    assert!(get.is_some(), "Expected call to 'get'");
    assert_eq!(
        get.unwrap().receiver_type.as_deref(),
        Some("DataCache"),
        "Local var 'cache' from 'new DataCache<string>()' should resolve receiver_type to 'DataCache' (stripped generics)"
    );
}

#[test]
fn test_ts_local_var_no_type_annotation() {
    let source = r#"class SomeService {
    doWork(): void {
        const result = this.calculate();
        result.process();
    }
    calculate(): any { return null; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let di = defs.iter().position(|d| d.name == "doWork").unwrap();
    let dc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == di).collect();
    assert!(!dc.is_empty(), "Expected call sites for 'doWork'");

    let process = dc[0].1.iter().find(|c| c.method_name == "process");
    assert!(process.is_some(), "Expected call to 'process'");
    assert_eq!(
        process.unwrap().receiver_type.as_deref(),
        Some("result"),
        "Local var 'result' with no type annotation and no new expression should preserve receiver name"
    );
}

#[test]
fn test_ts_local_var_field_types_take_precedence() {
    let source = r#"class MyComponent {
    private result: FieldType;

    doWork(): void {
        const result: LocalType = getValue();
        this.result.fieldMethod();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let di = defs.iter().position(|d| d.name == "doWork").unwrap();
    let dc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == di).collect();
    assert!(!dc.is_empty(), "Expected call sites for 'doWork'");

    let field_method = dc[0].1.iter().find(|c| c.method_name == "fieldMethod");
    assert!(field_method.is_some(), "Expected call to 'fieldMethod'");
    assert_eq!(
        field_method.unwrap().receiver_type.as_deref(),
        Some("FieldType"),
        "this.result.fieldMethod() should resolve to field type 'FieldType', not local var type 'LocalType'"
    );
}

// ─── TypeScript Local Variable Type — let Declaration Without Initializer ─────

#[test]
fn test_ts_local_var_let_declaration_without_initializer() {
    let source = r#"class TestClass {
    process(): void {
        let task: DependencyTask;
        task = this.createTask();
        task.resolve();
    }
    createTask(): any { return null; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process'");

    let resolve = pc[0].1.iter().find(|c| c.method_name == "resolve");
    assert!(resolve.is_some(), "Expected call to 'resolve'");
    assert_eq!(
        resolve.unwrap().receiver_type.as_deref(),
        Some("DependencyTask"),
        "Local var 'task' declared as 'let task: DependencyTask' (no initializer) should resolve receiver_type to 'DependencyTask'"
    );
}

// ─── Lambda / Arrow Function Parsing Tests ───────────────────────────

#[test]
fn test_ts_arrow_function_in_argument_calls_captured() {
    let source = r#"class ItemProcessor {
    process() {
        items.forEach(item => item.validate());
        promise.then(result => result.transform());
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process'");

    let names: Vec<&str> = pc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"forEach"), "Expected call to 'forEach', got: {:?}", names);
    assert!(names.contains(&"validate"), "Expected call to 'validate' inside arrow function, got: {:?}", names);
    assert!(names.contains(&"then"), "Expected call to 'then', got: {:?}", names);
    assert!(names.contains(&"transform"), "Expected call to 'transform' inside arrow function, got: {:?}", names);
}

#[test]
fn test_ts_multiline_arrow_function_calls_captured() {
    let source = r#"class TaskRunner {
    execute() {
        tasks.map(t => {
            t.initialize();
            t.run();
            return t.getResult();
        });
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites, _) = parse_typescript_for_test(&mut parser, source, 0);

    let ei = defs.iter().position(|d| d.name == "execute").unwrap();
    let ec: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ei).collect();
    assert!(!ec.is_empty(), "Expected call sites for 'execute'");

    let names: Vec<&str> = ec[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"initialize"), "Expected call to 'initialize' inside multiline arrow function, got: {:?}", names);
    assert!(names.contains(&"run"), "Expected call to 'run' inside multiline arrow function, got: {:?}", names);
    assert!(names.contains(&"getResult"), "Expected call to 'getResult' inside multiline arrow function, got: {:?}", names);
}
// ─── Angular Template Metadata Tests ─────────────────────────────────

// B1: structured Angular component metadata tests

#[test]
fn test_ts_angular_component_records_are_ast_structured() {
    use super::parser_typescript::parse_typescript_definitions_with_components;
    use super::{AngularTemplateSource, StaticValue};

    let source = r#"const selectorName = 'dynamic-selector';
@Component({
    resolvedTemplateUrl: './wrong.html',
    selector: 'app-inline',
    template: `<app-child>{{value}}</app-child>`,
})
export class InlineComponent {}

@Component({ 'selector': "app-external", "templateUrl": '../external.html' })
class ExternalComponent {}

@Component({ selector: selectorName, template: `<app-${kind}></app-${kind}>` })
class DynamicComponent {}

// selector: 'commented', templateUrl: './commented.html'
@Component({ selector: 'app-missing' })
class MissingComponent {}

@Component(componentMetadata)
class OpaqueComponent {}

@Component({ ...componentMetadata, selector: 'app-spread' })
class SpreadComponent {}

@Component()
class EmptyComponent {}

@Component({ selector: 'app-\u0072oot', template: '<app-\x63hild></app-\x63hild>' })
class EscapedComponent {}

@Component({ selector: 'app-first-same-line' }) class SameLineComponent {} @Component({ selector: 'app-second-same-line' }) class SameLineComponent {}"#;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let ((definitions, _, _), components) =
        parse_typescript_definitions_with_components(&mut parser, source, 0);
    let component = |name: &str| {
        components
            .iter()
            .find(|record| definitions[record.local_def_index].name == name)
            .unwrap_or_else(|| panic!("missing component record for {name}"))
    };

    assert_eq!(
        component("InlineComponent").component.selector,
        StaticValue::Static("app-inline".to_string())
    );
    assert_eq!(
        component("InlineComponent").component.template,
        AngularTemplateSource::Inline {
            content: "<app-child>{{value}}</app-child>".to_string(),
        }
    );
    assert_eq!(
        component("ExternalComponent").component,
        super::AngularComponentRecord {
            selector: StaticValue::Static("app-external".to_string()),
            template: AngularTemplateSource::External {
                relative_path: "../external.html".to_string(),
            },
        }
    );
    assert!(matches!(
        component("DynamicComponent").component.selector,
        StaticValue::Dynamic { .. }
    ));
    assert!(matches!(
        component("DynamicComponent").component.template,
        AngularTemplateSource::Dynamic { .. }
    ));
    assert_eq!(
        component("MissingComponent").component.template,
        AngularTemplateSource::Missing
    );
    for name in ["OpaqueComponent", "SpreadComponent"] {
        assert!(matches!(
            component(name).component.selector,
            StaticValue::Dynamic { .. }
        ));
        assert!(matches!(
            component(name).component.template,
            AngularTemplateSource::Dynamic { .. }
        ));
    }
    assert_eq!(
        component("EmptyComponent").component,
        super::AngularComponentRecord {
            selector: StaticValue::Missing,
            template: AngularTemplateSource::Missing,
        }
    );
    assert_eq!(
        component("EscapedComponent").component,
        super::AngularComponentRecord {
            selector: StaticValue::Static("app-root".to_string()),
            template: AngularTemplateSource::Inline {
                content: "<app-child></app-child>".to_string(),
            },
        }
    );
    let same_line_records: Vec<_> = components
        .iter()
        .filter(|record| definitions[record.local_def_index].name == "SameLineComponent")
        .collect();
    assert_eq!(same_line_records.len(), 2);
    assert_ne!(
        same_line_records[0].local_def_index,
        same_line_records[1].local_def_index
    );
    assert_eq!(
        same_line_records
            .iter()
            .map(|record| &record.component.selector)
            .collect::<Vec<_>>(),
        vec![
            &StaticValue::Static("app-first-same-line".to_string()),
            &StaticValue::Static("app-second-same-line".to_string()),
        ]
    );
}


#[test]
fn test_ts_angular_string_decoder_is_exact_and_panic_safe() {
    use super::parser_typescript::decode_ts_string_literal;

    assert_eq!(decode_ts_string_literal("'"), None);
    assert_eq!(
        decode_ts_string_literal(r#"'app-\u0072oot-\x31-\u{1f642}'"#),
        Some("app-root-1-🙂".to_string())
    );
    assert_eq!(decode_ts_string_literal(r#"'\uD800'"#), None);
    assert_eq!(decode_ts_string_literal(r#"'\8'"#), None);
    assert_eq!(decode_ts_string_literal(r#"'\01'"#), None);
}


#[test]
fn test_angular_template_path_normalization_stays_inside_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let nested = workspace.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let files = vec![nested.join("component.ts").to_string_lossy().to_string()];

    let (_, inside_key) = super::resolve_angular_template_path(
        &workspace.to_string_lossy(),
        &files,
        0,
        "../shared.html",
    )
    .expect("in-workspace parent path should resolve");
    let expected = crate::clean_path(
        &crate::path_identity_key(&workspace.join("shared.html")).to_string_lossy(),
    );
    assert_eq!(inside_key, expected);
    assert!(super::resolve_angular_template_path(
        &workspace.to_string_lossy(),
        &files,
        0,
        "../../outside.html",
    )
    .is_none());
}


#[test]
fn test_definition_index_builds_angular_records_and_derived_indexes() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    std::fs::write(
        root.join("components.ts"),
        r#"@Component({ selector: 'app-leaf' })
export class LeafComponent {}
@Component({ selector: 'app-inline', template: '<app-leaf></app-leaf><!-- <app-hidden></app-hidden> -->' })
export class InlineComponent {}
@Component({ selector: 'app-external', templateUrl: './external.html' })
export class ExternalComponent {}
@Component({ selector: dynamicSelector, template: makeTemplate() })
export class DynamicComponent {}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("external.html"),
        "<app-leaf></app-leaf><app-inline />",
    )
    .unwrap();

    let index = build_definition_index(&DefIndexArgs {
        dir: root.to_string_lossy().to_string(),
        ext: "ts".to_string(),
        threads: 1,
        respect_git_exclude: false,
    });
    let definition_index = |name: &str| {
        index
            .definitions
            .iter()
            .position(|definition| definition.name == name)
            .unwrap() as u32
    };
    let leaf = definition_index("LeafComponent");
    let inline = definition_index("InlineComponent");
    let external = definition_index("ExternalComponent");
    let dynamic = definition_index("DynamicComponent");

    assert_eq!(index.angular_components.len(), 4);
    assert!(matches!(
        index.angular_components[&dynamic].selector,
        StaticValue::Dynamic { .. }
    ));
    assert!(matches!(
        index.angular_components[&dynamic].template,
        AngularTemplateSource::Dynamic { .. }
    ));
    assert_eq!(index.selector_index["app-leaf"], vec![leaf]);
    assert_eq!(index.selector_index["app-inline"], vec![inline]);
    assert_eq!(index.selector_index["app-external"], vec![external]);
    assert_eq!(index.template_children[&inline], vec!["app-leaf"]);
    assert_eq!(
        index.template_children[&external],
        vec!["app-inline".to_string(), "app-leaf".to_string()]
    );
    assert_eq!(
        index.template_parents["app-leaf"],
        vec![inline, external]
    );
    assert_eq!(index.template_parents["app-inline"], vec![external]);
    assert!(!index.template_parents.contains_key("app-hidden"));

    let owner_key = crate::clean_path(
        &crate::path_identity_key(&root.join("external.html")).to_string_lossy(),
    );
    assert_eq!(index.template_owners[&owner_key], vec![external]);
}

#[test]
fn test_incremental_update_populates_angular_component_indexes() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    let path = root.join("component.ts");
    std::fs::write(
        &path,
        r#"@Component({ selector: 'app-parent', template: '<app-child></app-child>' })
export class ParentComponent {}"#,
    )
    .unwrap();

    let mut index = DefinitionIndex::default();
    update_file_definitions(&mut index, &path);

    let parent = index
        .definitions
        .iter()
        .position(|definition| definition.name == "ParentComponent")
        .unwrap() as u32;
    assert!(index.angular_components.contains_key(&parent));
    assert_eq!(index.selector_index["app-parent"], vec![parent]);
    assert_eq!(index.template_children[&parent], vec!["app-child"]);
    assert_eq!(index.template_parents["app-child"], vec![parent]);
}

#[test]
fn test_periodic_reconcile_preserves_transient_input_and_applies_stable_peer() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    let busy_path = root.join("busy.ts");
    let stable_path = root.join("stable.ts");
    std::fs::write(
        &busy_path,
        "@Component({ selector: 'app-busy-old' }) export class Busy {}",
    )
    .unwrap();
    std::fs::write(
        &stable_path,
        "@Component({ selector: 'app-stable-old' }) export class Stable {}",
    )
    .unwrap();
    let mut index = build_definition_index(&DefIndexArgs {
        dir: root.to_string_lossy().to_string(),
        ext: "ts".to_string(),
        threads: 1,
        respect_git_exclude: false,
    });
    index.created_at = 0;
    let generation = index.definition_generation;
    let index = std::sync::Arc::new(std::sync::RwLock::new(index));
    std::fs::write(
        &busy_path,
        "@Component({ selector: 'app-busy-new' }) export class Busy {}",
    )
    .unwrap();
    std::fs::write(
        &stable_path,
        "@Component({ selector: 'app-stable-new' }) export class Stable {}",
    )
    .unwrap();
    super::install_definition_source_read_error(
        &busy_path,
        std::io::ErrorKind::WouldBlock,
    );

    let (_, modified, _) = reconcile_definition_index_nonblocking(
        &index,
        &root.to_string_lossy(),
        &["ts".to_string()],
        false,
    );

    assert!(modified > 0);
    {
        let mut index = index.write().unwrap();
        assert_eq!(index.definition_generation, generation + 1);
        assert!(index.created_at > 0, "global watermark must advance");
        assert!(index.pending_definition_inputs.contains_key(
            &crate::path_identity_key(&busy_path)
        ));
        assert!(index
            .path_to_id
            .contains_key(&crate::path_identity_key(&busy_path)));
        assert!(index.selector_index.contains_key("app-busy-old"));
        assert!(!index.selector_index.contains_key("app-busy-new"));
        assert!(!index.selector_index.contains_key("app-stable-old"));
        assert!(index.selector_index.contains_key("app-stable-new"));
        index.created_at = index.created_at.saturating_add(3600);
    }

    super::remove_definition_source_read_error(&busy_path);
    let (_, modified, _) = reconcile_definition_index_nonblocking(
        &index,
        &root.to_string_lossy(),
        &["ts".to_string()],
        false,
    );
    assert_eq!(modified, 1, "pending source must retry independently of mtime");
    let index = index.read().unwrap();
    assert_eq!(index.definition_generation, generation + 2);
    assert!(index.pending_definition_inputs.is_empty());
    assert!(!index.selector_index.contains_key("app-busy-old"));
    assert!(index.selector_index.contains_key("app-busy-new"));
}

#[test]
fn test_periodic_permission_denied_quarantine_stops_new_file_retry_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    let path = root.join("component.ts");
    std::fs::write(
        &path,
        "@Component({ selector: 'app-child' }) export class Component {}",
    )
    .unwrap();
    let mut index = build_definition_index(&DefIndexArgs {
        dir: root.to_string_lossy().to_string(),
        ext: "ts".to_string(),
        threads: 1,
        respect_git_exclude: false,
    });
    index.created_at = 0;
    let generation = index.definition_generation;
    let index = std::sync::Arc::new(std::sync::RwLock::new(index));
    super::install_definition_source_read_error(
        &path,
        std::io::ErrorKind::PermissionDenied,
    );

    let mut exhausted = (0, 0, 0);
    for _ in 0..super::MAX_TRANSIENT_DEFINITION_ATTEMPTS {
        exhausted = reconcile_definition_index_nonblocking(
            &index,
            &root.to_string_lossy(),
            &["ts".to_string()],
            false,
        );
    }
    let no_op = reconcile_definition_index_nonblocking(
        &index,
        &root.to_string_lossy(),
        &["ts".to_string()],
        false,
    );
    super::remove_definition_source_read_error(&path);

    assert_eq!(exhausted, (0, 0, 1), "quarantine tombstone is a removal");
    assert_eq!(no_op, (0, 0, 0), "quarantined path must not restart retries");
    let index_guard = index.read().unwrap();
    assert_eq!(index_guard.definition_generation, generation + 1);
    assert_eq!(
        index_guard
            .pending_definition_inputs
            .get(&crate::path_identity_key(&path))
            .map(|pending| pending.attempts),
        Some(super::MAX_TRANSIENT_DEFINITION_ATTEMPTS)
    );
    assert!(!index_guard
        .path_to_id
        .contains_key(&crate::path_identity_key(&path)));
    assert!(!index_guard.selector_index.contains_key("app-child"));
    drop(index_guard);

    std::fs::write(
        &path,
        "@Component({ selector: 'app-recovered' }) export class Component {} // changed",
    )
    .unwrap();
    let (added, _, _) = reconcile_definition_index_nonblocking(
        &index,
        &root.to_string_lossy(),
        &["ts".to_string()],
        false,
    );
    assert_eq!(added, 1, "revision change must reactivate quarantine");
    let index = index.read().unwrap();
    assert!(index.pending_definition_inputs.is_empty());
    assert!(index.selector_index.contains_key("app-recovered"));
}



// B2: extract_custom_elements tests

#[test]
fn test_extract_custom_elements_basic() {
    let html = "<div><my-component></my-component><span>text</span></div>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["my-component"]);
}

#[test]
fn test_extract_custom_elements_self_closing() {
    let html = "<my-widget /><another-comp/>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["another-comp", "my-widget"]);
}

#[test]
fn test_extract_custom_elements_ignores_orphan_closing_custom_tag() {
    let html = "</app-orphan><app-active></app-active>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_with_attributes() {
    let html = r#"<my-comp [input]="value" (output)="handler($event)"></my-comp>"#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["my-comp"]);
}

#[test]
fn test_extract_custom_elements_excludes_standard_html() {
    let html = "<div><span><p><h1><input><br><table><tr><td></td></tr></table></h1></p></span></div>";
    let result = super::extract_custom_elements(html);
    assert!(result.is_empty());
}

#[test]
fn test_extract_custom_elements_excludes_ng_builtins() {
    let html = "<ng-container><ng-content></ng-content><ng-template></ng-template></ng-container>";
    let result = super::extract_custom_elements(html);
    assert!(result.is_empty());
}

#[test]
fn test_extract_custom_elements_dedup_and_case_insensitive() {
    let html = "<My-Component></My-Component><my-component></my-component><MY-COMPONENT></MY-COMPONENT>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["my-component"]);
}

#[test]
fn test_extract_custom_elements_empty_html() {
    let result = super::extract_custom_elements("");
    assert!(result.is_empty());
}

#[test]
fn test_extract_custom_elements_ignores_html_comments() {
    let html = "<!-- <app-ghost></app-ghost> -->";
    let result = super::extract_custom_elements(html);
    assert!(result.is_empty());
}

#[test]
fn test_extract_custom_elements_keeps_active_tags_around_comments() {
    let html = "<app-before></app-before><!-- <app-hidden> --><app-after></app-after>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-after", "app-before"]);
}

#[test]
fn test_extract_custom_elements_recovers_from_less_than_in_angular_expression() {
    let html = "@if (a<b) { <app-child></app-child> }";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-child"]);
}

#[test]
fn test_extract_custom_elements_ignores_multiple_multiline_comments() {
    let html = r#"
        <!--
            <app-first-hidden></app-first-hidden>
        -->
        <app-active></app-active>
        <!--
            <app-second-hidden></app-second-hidden>
        -->
    "#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_ignores_unclosed_comment_to_eof() {
    let html = "<app-active></app-active><!-- <app-hidden></app-hidden>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_ignores_declarations_and_cdata() {
    let html = r#"<!DOCTYPE html PUBLIC "<app-doctype>" "about:legacy-compat">
        <![CDATA[<app-cdata></app-cdata>]]>
        <app-active></app-active>
    "#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_keeps_cdata_inactive_after_greater_than() {
    let html = "<![CDATA[x > <app-hidden></app-hidden>]]><app-active></app-active>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_keeps_quoted_declaration_text_inactive() {
    let html = r#"<!DOCTYPE html PUBLIC "x > <app-hidden>"><app-active></app-active>"#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_ignores_processing_instructions() {
    let html = r#"<?php echo "<app-hidden>"; ?><app-active></app-active>"#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_ignores_tags_in_quoted_attributes() {
    let html = r#"<div data-example="<app-hidden></app-hidden>"></div>
        <app-active label='<app-also-hidden>'></app-active>
    "#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_does_not_end_tag_inside_quoted_attribute() {
    let html = r#"<app-a title="><app-b"></app-a>"#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-a"]);
}

#[test]
fn test_extract_custom_elements_ignores_script_and_style_raw_text() {
    let html = r#"<script>const example = "<app-script></app-script>";</script>
        <style>.example::before { content: "<app-style>"; }</style>
        <app-active></app-active>
    "#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_requires_raw_text_close_tag_boundary() {
    let html = "<script></scriptx><app-hidden></app-hidden></script><app-active></app-active>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_keeps_self_closing_raw_text_tag_in_data_state() {
    let html = "<script/><style /><app-active></app-active>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_keeps_unmatched_raw_text_closers_in_data_state() {
    let html = "</script><app-script-child></app-script-child></style><app-style-child></app-style-child>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-script-child", "app-style-child"]);
}

#[test]
fn test_extract_custom_elements_redispatches_repeated_tag_open() {
    let html = "<<app-active></app-active>";
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-active"]);
}

#[test]
fn test_extract_custom_elements_ignores_unclosed_inactive_contexts() {
    let cases = [
        "<![CDATA[<app-hidden>",
        r#"<div title="<app-hidden>"#,
        r#"<!DOCTYPE html PUBLIC "<app-hidden>""#,
        "<script><app-hidden>",
        "<style><app-hidden>",
        r#"<app-unclosed title="value"#,
    ];

    for html in cases {
        let result = super::extract_custom_elements(html);
        assert!(result.is_empty(), "unexpected elements for {html:?}: {result:?}");
    }
}

#[test]
fn test_extract_custom_elements_skips_templates_above_source_parse_limit() {
    let limit = super::MAX_PARSE_SOURCE_BYTES;
    let html = format!("<app-hidden>{}", "x".repeat(limit));
    assert!(html.len() > limit);

    let result = super::extract_custom_elements(&html);
    assert!(result.is_empty());
}

#[test]
fn test_extract_custom_elements_accepts_template_at_source_parse_limit() {
    let limit = super::MAX_PARSE_SOURCE_BYTES;
    let tag = "<app-at-limit></app-at-limit>";
    let html = format!("{tag}{}", "x".repeat(limit - tag.len()));
    assert_eq!(html.len(), limit);

    let result = super::extract_custom_elements(&html);
    assert_eq!(result, vec!["app-at-limit"]);
}

#[test]
fn test_read_angular_template_enforces_source_parse_limit() {
    let temp = tempfile::tempdir().unwrap();
    let template = temp.path().join("component.html");
    let limit = super::MAX_PARSE_SOURCE_BYTES;

    std::fs::write(&template, vec![b'x'; limit]).unwrap();
    match super::read_angular_template(&template).unwrap() {
        super::AngularTemplateRead::Content { content, .. } => {
            assert_eq!(content.len(), limit)
        }
        super::AngularTemplateRead::TooLarge { observed_size } => {
            panic!("exact-limit template was rejected at {observed_size} bytes")
        }
    }

    std::fs::write(&template, vec![b'x'; limit + 1]).unwrap();
    match super::read_angular_template(&template).unwrap() {
        super::AngularTemplateRead::TooLarge { observed_size } => {
            assert_eq!(observed_size, (limit + 1) as u64)
        }
        super::AngularTemplateRead::Content { .. } => {
            panic!("oversized template was read")
        }
    }
}

#[test]
fn test_extract_custom_elements_mixed() {
    let html = r#"
        <div class="container">
            <ng-container *ngIf="show">
                <data-grid [config]="gridConfig"></data-grid>
                <app-spinner size="large"></app-spinner>
                <span>Loading...</span>
            </ng-container>
            <app-footer></app-footer>
        </div>
    "#;
    let result = super::extract_custom_elements(html);
    assert_eq!(result, vec!["app-footer", "app-spinner", "data-grid"]);
}

// ─── Enum with explicit values (enum_assignment) regression tests ────

#[test]
fn test_parse_ts_enum_with_string_values() {
    let source = r#"export enum TemplateName {
    Report = "report",
    Dashboard = "dashboard",
    ReportVisual = "reportVisual"
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _, _) = parse_typescript_for_test(&mut parser, source, 0);

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "TemplateName");

    let members: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(members.len(), 3, "Expected 3 enum members for enum with string values");
    let names: Vec<&str> = members.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Report"));
    assert!(names.contains(&"Dashboard"));
    assert!(names.contains(&"ReportVisual"));
    for m in &members {
        assert_eq!(m.parent.as_deref(), Some("TemplateName"),
            "Enum member '{}' should have parent 'TemplateName'", m.name);
        // enum_assignment members should have signature with the value
        assert!(m.signature.is_some(),
            "Enum member '{}' should have signature", m.name);
    }
}

#[test]
fn test_parse_ts_enum_with_numeric_values() {
    let source = r#"enum Priority {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _, _) = parse_typescript_for_test(&mut parser, source, 0);

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "Priority");

    let members: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(members.len(), 4, "Expected 4 enum members for numeric enum");
    let names: Vec<&str> = members.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Low"));
    assert!(names.contains(&"Medium"));
    assert!(names.contains(&"High"));
    assert!(names.contains(&"Critical"));
    for m in &members {
        assert_eq!(m.parent.as_deref(), Some("Priority"));
    }
}

#[test]
fn test_parse_ts_enum_mixed_members() {
    // Mix of plain identifiers and enum_assignment nodes
    let source = r#"enum MixedEnum {
    Auto,
    Manual = "manual",
    Default = 0
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _, _) = parse_typescript_for_test(&mut parser, source, 0);

    let members: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(members.len(), 3, "Expected 3 enum members for mixed enum");
    let names: Vec<&str> = members.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Auto"));
    assert!(names.contains(&"Manual"));
    assert!(names.contains(&"Default"));
}
