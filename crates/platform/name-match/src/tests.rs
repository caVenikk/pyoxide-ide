//! Cases of the reference IDE's matcher tests whose behaviour is clear.

use super::*;

fn matcher(pattern: &str, mode: MatchingMode) -> Matcher {
    Matcher::new(pattern, mode)
}

#[track_caller]
fn assert_matches(pattern: &str, name: &str) {
    assert!(
        matcher(pattern, MatchingMode::IgnoreCase).matches(name),
        "{pattern} doesn't match {name}"
    );
}

#[track_caller]
fn assert_doesnt_match(pattern: &str, name: &str) {
    assert!(
        !matcher(pattern, MatchingMode::IgnoreCase).matches(name),
        "{pattern} matches {name}"
    );
}

#[track_caller]
fn first_letter(pattern: &str, name: &str) -> bool {
    matcher(pattern, MatchingMode::FirstLetter).matches(name)
}

#[track_caller]
fn case_sensitive(pattern: &str, name: &str) -> bool {
    matcher(pattern, MatchingMode::MatchCase).matches(name)
}

#[track_caller]
fn assert_preference_in(pattern: &str, less: &str, more: &str, mode: MatchingMode) {
    let m = matcher(pattern, mode);
    let (l, r) = (m.degree(less), m.degree(more));
    assert!(l < r, "{pattern}: {less} {l} >= {more} {r}");
}

#[track_caller]
fn assert_preference(pattern: &str, less: &str, more: &str) {
    assert_preference_in(pattern, less, more, MatchingMode::FirstLetter);
}

#[track_caller]
fn assert_no_preference(pattern: &str, a: &str, b: &str, mode: MatchingMode) {
    let m = matcher(pattern, mode);
    assert_eq!(m.degree(a), m.degree(b), "{pattern}: {a} vs {b}");
}

fn fragments(pattern: &str, name: &str) -> Option<Vec<Range<usize>>> {
    matcher(pattern, MatchingMode::IgnoreCase).find(name)
}

#[test]
fn simple_cases() {
    assert_matches("N", "NameUtilTest");
    assert_matches("NU", "NameUtilTest");
    assert_matches("NUT", "NameUtilTest");
    assert_matches("NaUT", "NameUtilTest");
    assert_doesnt_match("NeUT", "NameUtilTest");
    assert_doesnt_match("NaUTa", "NameUtilTest");
    assert_matches("NaUtT", "NameUtilTest");
    assert_matches("NaUtTe", "NameUtilTest");
    assert_matches("AACl", "AAClass");
    assert_matches("ZZZ", "ZZZZZZZZZZ");
    assert_matches("", "");
    assert_matches("", "asdfs");
}

#[test]
fn skip_words() {
    assert_matches("nt", "NameUtilTest");
    assert_matches("repl map", "ReplacePathToMacroMap");
    assert_matches("replmap", "ReplacePathToMacroMap");
    assert_matches("CertificateEx", "CertificateEncodingException");
    assert_doesnt_match("ABCD", "AbstractButton.DISABLED_ICON_CHANGED_PROPERTY");
    assert_matches("templipa", "template_impl_template_list_panel");
    assert_matches("templistpa", "template_impl_template_list_panel");
}

#[test]
fn first_letter_lower_case() {
    assert_matches("N", "nameUtilTest");
    assert_doesnt_match("N", "anameUtilTest");
    assert_matches("NU", "nameUtilTest");
    assert_doesnt_match("NU", "anameUtilTest");
    assert_matches("NaUtTe", "nameUtilTest");
    assert_doesnt_match("NeUT", "nameUtilTest");
}

#[test]
fn space_delimiters() {
    assert_matches("Na Ut Te", "name util test");
    assert_matches("Na Ut Te", "name Util Test");
    assert_doesnt_match("Na Ut Ta", "name Util Test");
    assert_matches("na ut te", "name util test");
    assert_matches("na ut", "name_util_test");
    assert_matches("na te", "name_util_test");
    assert_doesnt_match("na ti", "name_util_test");
    assert_doesnt_match("alias imple", "alias simple");
    assert_doesnt_match("alias mple", "alias simple");
    assert_doesnt_match("alias nother", "alias another");
}

#[test]
fn underscores_and_capitals() {
    assert_doesnt_match("N_T", "NameUtilTest");
    assert_matches("ORGS_ACC", "ORGS_POSITION_ACCOUNTABILITY");
    assert_matches("ORGS-ACC", "ORGS-POSITION_ACCOUNTABILITY");
    assert_matches("ORGS.ACC", "ORGS.POSITION_ACCOUNTABILITY");
    assert_doesnt_match("ar*l*p", "AbstractResponseHandler");
    assert_matches("N_U_T", "NAME_UTIL_TEST");
    assert_matches("NUT", "NAME_UTIL_TEST");
    assert_doesnt_match("NUT", "NameutilTest");
    assert_matches("NOS", "NetOutputStream");
}

#[test]
fn file_name_conventions() {
    assert_matches("BLWN", "base_layout_without_navigation.xhtml");
    assert_matches("BLWN", "base-layout-without-navigation.xhtml");
    assert_matches("FC", "faces-config.xml");
    assert_matches("ARS", "activity_report_summary.jsp");
    assert_matches("AD", "arrow_down.gif");
    assert_matches("VL", "vehicle-listings.css");
    assert_matches("ARS.j", "activity_report_summary.jsp");
    assert_doesnt_match("ARS.j", "activity_report_summary.xml");
    assert_doesnt_match("ARS.j", "activity_report_summary_justsometingwrong.xml");
    assert_matches("foo.goo", "foo.bar.goo");
    assert_doesnt_match("*.ico", "sm.th.iks.concierge");
    assert_matches("AR.jsp", "add_relationship.jsp");
    assert_matches("AR.jsp", "AlarmRuleSeverity.jsp");
    assert_matches("AR.jsp", "audiot_r.jsp");
}

#[test]
fn spaces_dots_and_extensions() {
    assert_matches("fo bar", "fooBar");
    assert_matches("foo bar", "fooGooBar");
    assert_matches("foo bar", "fooGoo bar");
    assert_doesnt_match(" b", "fbi");
    assert_doesnt_match(" for", "performAction");
    assert!(matcher(" us", MatchingMode::IgnoreCase).matches("getUsage"));
    assert_matches("Google Test.html", "Google Test Test.cc.html");
    assert_matches("Google.html", "Google Test Test.cc.html");
    assert_matches("Google Test*.html", "Google Test Test.cc.html");
    assert_matches("A*.html", "A.html");
    assert_matches("A*.html", "after.html");
    assert_doesnt_match("A*.html", "10_after.html");
    assert_matches("ja", "jquery.autocomplete.js");
    assert_doesnt_match("ja.js", "jquery.autocomplete.js");
    assert_matches("jajs", "jquery.autocomplete.js");
    assert_matches("j.js", "jquery.autocomplete.js");
    assert_doesnt_match("j.ajs", "jquery.autocomplete.js");
    assert_matches("oracle.bnf", "oracle-11.2.bnf");
    assert_matches("*foo.*bar", "foo.b.bar");
    assert_matches("#.p", "#.php");
    assert_matches("a", "a.php");
    assert_matches("comboBox", "combobox");
    assert_matches(".foo", ".foo");
    assert_matches("*inspection*.pro", "InspectionsBundle.properties");
    assert_doesnt_match("*inspection*.pro", "InspectionsInProgress.png");
    assert_doesnt_match("form", "_form.html.erb");
    assert_matches("_form", "_form.html.erb");
    assert_matches("_form", "__form");
}

#[test]
fn lower_case_words() {
    assert_matches("uct", "unit_controller_test");
    assert_matches("unictest", "unit_controller_test");
    assert_doesnt_match("nc", "unit_controller_test");
    assert_doesnt_match("utc", "unit_controller_test");
    assert_matches("foo", "fxOo");
    assert_doesnt_match("foo", "fxoo");
    assert_doesnt_match("fOo", "FaOaOaXXXX");
    assert_matches("ncdfoe", "NoClassDefFoundException");
    assert_matches("fob", "FOO_BAR");
    assert_matches("fo b", "FOO BAR");
    assert_matches("buico.gr", "BuildConfig.groovy");
    assert_matches("bui.gr", "BuildConfig.groovy");
    assert_matches("*fz", "azzzfzzz");
    assert_matches("WebLOgic", "WebLogic");
    assert_doesnt_match("WebLogic", "Webologic");
    assert_matches("Wlo", "WebLogic");
    assert_matches("LSTMa", "LineStatusTrackerManager");
}

#[test]
fn middle_matching() {
    assert_matches("*fo", "reformat");
    assert_matches("*f", "reFormat");
    assert_matches("*Stri", "string");
    assert_doesnt_match("*fc", "reformatCode");
    assert_doesnt_match("*foc", "reformatCode");
    assert_matches("*forc", "reformatCode");
    assert_doesnt_match("*sTC", "LazyClassTypeConstructor");
    assert_doesnt_match("*Icon", "LEADING_CONSTRUCTOR");
    assert_matches("*ing", "LEADING_CONSTRUCTOR");
    assert_doesnt_match("*inc", "LEADING_CONSTRUCTOR");
    assert_doesnt_match("*ico", "drawLinePickedOut");
    assert_matches("*leg", "AppDelegate");
    assert_matches("*_dark.png", "collapseAll_dark.png");
    assert_matches("**_dark.png", "collapseAll_dark.png");
    assert_matches("*zz*", "ListConfigzzKey");
    assert_matches("SWU*H*7", "SWUpgradeHdlrFSPR7Test");
    assert_matches("SWU*H*R", "SWUPGRADEHDLRFSPR7TEST");
    assert_doesnt_match("*get*A", "getClass");
    assert_matches("*Git", "BlaGitBla");
    assert_matches("*git", "BlagitBla");
    assert_matches("Collec*Util*", "CollectionUtilsTest");
    assert_matches(" File. sepa", "File._separator");
    assert_matches("*BComp", "BaseComponent");
    assert_matches("*OS", "ios");
    assert_matches("*LRUMap", "SLRUMap");
    assert_matches(
        "*pro",
        "spring.activemq.pool.configuration.reconnect-on-exception",
    );
}

#[test]
fn first_letter_and_case_sensitive_modes() {
    assert!(first_letter("*I", "ID"));
    assert!(!first_letter("*I", "id"));
    assert!(first_letter(" cl", "getClass"));
    assert!(!first_letter(" EUC-", "x-EUC-TW"));
    assert!(!first_letter(" a", "Aaa"));
    assert!(!first_letter("*Ch", "char"));
    assert!(first_letter("*Codes", "CFLocaleCopyISOCountryCodes"));
    assert!(!first_letter("*codes", "CFLocaleCopyISOCountryCodes"));
    assert!(first_letter("WebLogic", "WebLogic"));
    assert!(!first_letter("webLogic", "WebLogic"));
    assert!(first_letter("cL", "class"));
    assert!(first_letter("Cl", "CoreLoader"));
    assert!(!first_letter("abc", "_abc"));
    assert!(case_sensitive("WebLogic", "WebLogic"));
    assert!(!case_sensitive("webLogic", "weblogic"));
    assert!(!case_sensitive("FOO", "foo"));
    assert!(!case_sensitive("Wl", "WebLogic"));
    assert!(case_sensitive("WL", "WebLogic"));
    assert!(!case_sensitive("WL", "Weblogic"));
    assert!(!case_sensitive("*list*", "AbstractList"));
}

#[test]
fn final_space_and_digits() {
    assert_matches("a ", "alpha + beta");
    assert_matches("a ", "a");
    assert_matches("GrDebT ", "GroovyDebuggerTest");
    assert_doesnt_match("grdebT ", "GroovyDebuggerTest");
    assert_matches("Foo ", "Foo");
    assert_doesnt_match("Foo ", "FooBar");
    assert_doesnt_match("Collections ", "CollectionSplitter");
    assert_matches("CollectionS ", "CollectionSplitter");
    assert_matches("*run ", "in Runnable.run");
    assert_doesnt_match("*leg ", "AppDelegate");
    assert_matches("foba4", "FooBar4");
    assert_matches("foba", "Foo4Bar");
    assert_matches("*TEST-0* ", "TEST-001");
    assert_matches("*v2 ", "VARCHAR2");
    assert_matches("smart8co", "SmartType18CompletionTest");
    assert_doesnt_match("*012", "001122");
    assert_matches("012", "0a1_22");
    assert_matches("a@b", "a@bc");
    assert_doesnt_match("*@in", "a int");
    assert_matches("a/text", "a/bbbText");
    assert_matches("a+b", "alpha + beta");
    assert_doesnt_match("a ", "alpha+beta");
    assert_doesnt_match("d+g", "alphaDelta+betaGamma");
    assert_matches("*d+g", "alphaDelta+betaGamma");
    assert_matches("groovy*.jar", "groovy-1.7.jar");
    assert_doesnt_match("*.ico", "a.i.c.o");
    assert_matches("PDFRe", "PdfRenderer");
    assert_matches("NSUSerDefa", "NSUserDefaults");
    assert_matches("NSUSD", "NSUserDefaults");
}

#[test]
fn matched_fragments() {
    let sample = "NoClassDefFoundException";
    assert_eq!(
        fragments("ncldfou*ion", sample),
        Some(vec![0..1, 2..4, 7..8, 10..13, 21..24])
    );
    let sample = "doGet(HttpServletRequest, HttpServletResponse):void";
    assert_eq!(fragments("d*st", sample), Some(vec![0..1, 22..24]));
    assert_eq!(fragments("doge*st", sample), Some(vec![0..4, 22..24]));
    assert_eq!(fragments("_t", "_test"), Some(vec![0..2]));
    assert_eq!(
        fragments("SWU*H*R", "SWUPGRADEHDLRFSPR7TEST"),
        Some(vec![0..3, 9..10, 12..13])
    );
    assert_eq!(
        fragments("getCU", "getCurrentUser"),
        Some(vec![0..4, 10..11])
    );
    assert_eq!(
        fragments("GEN", "GetExtendedName").map(|f| f.len()),
        Some(3)
    );
}

#[test]
fn matching_degree() {
    use MatchingMode::IgnoreCase;
    assert_preference_in(
        "jscote",
        "JsfCompletionTest",
        "JSCompletionTest",
        IgnoreCase,
    );
    assert_preference("OCO", "OneCoolObject", "OCObject");
    assert_preference("MUp", "MavenUmlProvider", "MarkUp");
    assert_preference("MUP", "MarkUp", "MavenUmlProvider");
    assert_preference(
        "CertificateExce",
        "CertificateEncodingException",
        "CertificateException",
    );
    assert_preference("getCU", "getCurrentSomething", "getCurrentUser");
    assert_preference("cL", "class", "coreLoader");
    assert_preference_in("inse", "InstrumentationError", "intSet", IgnoreCase);
    assert_preference_in("String", "STRING", "String", IgnoreCase);
    assert_preference_in("*psfi", "PsiJavaFileBaseImpl", "PsiFileImpl", IgnoreCase);
    assert_preference_in("*e", "fileIndex", "file", IgnoreCase);
    assert_preference_in(" fb", "FooBar", "_fooBar", IgnoreCase);
    assert_preference("*foo", "barFoo", "foobar");
    assert_preference("*fo", "barfoo", "barFoo");
    assert_preference("*fo", "barfoo", "foo");
    assert_preference_in(
        " sto",
        "StackOverflowError",
        "ArrayStoreException",
        IgnoreCase,
    );
    assert_preference("ob", "oci_bind_array_by_name", "obj");
    assert_no_preference("en", "ENABLED", "Enum", IgnoreCase);
    assert_preference_in("*ap", "add_profile", "application", IgnoreCase);
    assert_preference_in("*ea", "LEADING", "NORTH_EAST", IgnoreCase);
    assert_preference_in("*icon", "isControlKeyDown", "getErrorIcon", IgnoreCase);
    assert_preference_in(
        "CBP",
        "CustomProcessBP",
        "ComputationBatchProcess",
        IgnoreCase,
    );
    assert_preference("DH", "DAHolder", "DataHolder");
    assert_preference("DH", "DAOHolder", "DAHolder");
    assert_no_preference(
        "PropComp",
        "PropertyComponent",
        "PropertiesComponent",
        IgnoreCase,
    );
    assert_no_preference(
        " path",
        "getAbsolutePath",
        "findPath",
        MatchingMode::FirstLetter,
    );
    assert_preference_in("*tree", "FooTree", "Tree", IgnoreCase);
    assert_preference_in(
        "*mappablejs",
        "mappable-js.scope.js",
        "MappableJs.js",
        IgnoreCase,
    );
    assert_preference("ProVi", "PROVIDER", "ProjectView");
    assert_preference("GEN", "GetName", "GetExtendedName");
    assert_preference("*GEN", "GetName", "GetExtendedName");
    assert!(matcher(" EUC-", MatchingMode::IgnoreCase).degree("x-EUC-TW") > i32::MIN);
}

#[test]
fn first_letter_case_counts_when_valued() {
    let m = matcher("*pim", MatchingMode::IgnoreCase);
    let name = |s: &str| s.chars().collect::<Vec<_>>();
    let less = m.match_with(&name("PImageDecoder"), true).unwrap().degree;
    let more = m.match_with(&name("posIdMap"), true).unwrap().degree;
    assert!(less < more);
}

#[test]
fn wrong_keyboard_layout() {
    assert_matches("ыек", "String");
    assert_eq!(fragments("ыек", "String"), Some(vec![0..3]));
}

#[test]
fn start_matches_are_preferred_on_request() {
    let plain = Matcher::new("*app", MatchingMode::IgnoreCase);
    let start = Matcher::new("*app", MatchingMode::IgnoreCase).prefer_start();
    assert_eq!(
        start.degree("app.py"),
        plain.degree("app.py") + START_MATCH_WEIGHT
    );
    assert_eq!(start.degree("myapp.py"), plain.degree("myapp.py"));
}

#[test]
fn long_patterns_match_as_prefixes() {
    let s = "the class with its attributes mapped to fields of records parsed by an {@link AbstractParser} or written by an {@link AbstractWriter}.";
    assert_eq!(fragments(s, s), Some(vec![0..s.chars().count()]));
    assert_matches(
        "Product.findByDateAndNameGreaterThanEqualsAndQualityGreaterThanEqual",
        "Product.findByDateAndNameGreaterThanEqualsAndQualityGreaterThanEqualsIntellijIdeaRulezzz",
    );
}

#[test]
fn word_prefixes_in_any_order() {
    let m = WordPrefixMatcher::new("usa fin");
    assert!(m.matches("Find Usages"));
    assert!(m.matches("Find Usages in File"));
    assert!(!m.matches("Find Uses"));
    assert!(WordPrefixMatcher::new("go file").matches("Go to File…"));
    assert!(WordPrefixMatcher::new("sh/ex").matches("Show Excluded"));
}
