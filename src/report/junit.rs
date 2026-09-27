use quick_junit::{
    NonSuccessKind, Report as Junit, SerializeError, TestCase, TestCaseStatus, TestSuite,
};

use crate::config::Unspecified;
use crate::lint::{LintReport, Rule, Severity};
use crate::runner::{Outcome, Report};

fn document(junit: &Junit) -> Result<String, SerializeError> {
    let mut out = junit.to_string()?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn non_success(kind: NonSuccessKind, message: &str, description: String) -> TestCaseStatus {
    let mut status = TestCaseStatus::non_success(kind);
    status.set_message(message).set_description(description);
    status
}

/// One suite per table, one test case per case (never folded); under `unspecified: fail`, a
/// `coverage` suite with a failing test case per table × identity that has gaps.
pub fn render(report: &Report) -> Result<String, SerializeError> {
    let mut junit = Junit::new("rlsspec test");
    let mut suites: Vec<TestSuite> = Vec::new();
    for result in &report.results {
        let location = format!(
            "{}:{}:{}",
            report.spec.display(),
            result.span.line,
            result.span.column
        );
        let located = |detail: &str| {
            if detail.contains(&location) {
                detail.to_owned()
            } else {
                format!("{detail}\nat {location}")
            }
        };
        let status = match &result.outcome {
            Outcome::Pass(_) => TestCaseStatus::success(),
            Outcome::Fail(detail) => non_success(NonSuccessKind::Failure, detail, located(detail)),
            Outcome::Inconclusive(detail) => {
                non_success(NonSuccessKind::Error, detail, located(detail))
            }
        };
        let name = format!(
            "{} {} {}",
            result.identity,
            result.op.as_str(),
            result.description
        );
        let mut case = TestCase::new(name, status);
        case.set_classname(result.table.as_str());
        match suites.iter_mut().find(|s| s.name.as_str() == result.table) {
            Some(suite) => {
                suite.add_test_case(case);
            }
            None => {
                let mut suite = TestSuite::new(result.table.as_str());
                suite.add_test_case(case);
                suites.push(suite);
            }
        }
    }

    let coverage = &report.coverage;
    if coverage.policy == Unspecified::Fail && coverage.unspecified() > 0 {
        let mut suite = TestSuite::new("coverage");
        for (table, gaps) in coverage.gaps() {
            for gap in gaps {
                let ops: Vec<&str> = gap.ops.iter().map(|op| op.as_str()).collect();
                let message = format!("unspecified: {}", ops.join(", "));
                let status = non_success(
                    NonSuccessKind::Failure,
                    &message,
                    format!("{message} (unspecified: fail)"),
                );
                let mut case = TestCase::new(format!("{} coverage", gap.identity), status);
                case.set_classname(table);
                suite.add_test_case(case);
            }
        }
        suites.push(suite);
    }
    junit.add_test_suites(suites);
    document(&junit)
}

/// One suite per rule. An error is a failing test case; a warning or info passes with its hint in
/// system-out, so the JUnit verdict matches the exit code. A rule with no finding has one passing
/// `no findings` case, a rule skipped on this server one skipped case. Ignored findings aren't listed.
pub fn render_lint(report: &LintReport) -> Result<String, SerializeError> {
    let mut junit = Junit::new("rlsspec lint");
    for rule in Rule::ALL {
        let mut suite = TestSuite::new(rule.id());
        let case = |name: &str, status| {
            let mut case = TestCase::new(name, status);
            case.set_classname(rule.id());
            case
        };
        if let Some(skipped) = report.skipped.iter().find(|s| s.rule == rule) {
            let mut status = TestCaseStatus::skipped();
            status.set_message(skipped.reason);
            suite.add_test_case(case("skipped", status));
        }
        for finding in report.findings.iter().filter(|f| f.rule == rule) {
            let name = match &finding.role {
                Some(role) => format!("{} {role}", finding.object),
                None => finding.object.clone(),
            };
            let text = format!("{}: {}", finding.severity.as_str(), finding.hint);
            let test = match finding.severity {
                Severity::Error => case(
                    &name,
                    non_success(NonSuccessKind::Failure, &finding.hint, text),
                ),
                Severity::Warn | Severity::Info => {
                    let mut test = case(&name, TestCaseStatus::success());
                    test.set_system_out(text);
                    test
                }
            };
            suite.add_test_case(test);
        }
        if suite.test_cases.is_empty() {
            suite.add_test_case(case("no findings", TestCaseStatus::success()));
        }
        junit.add_test_suite(suite);
    }
    document(&junit)
}
