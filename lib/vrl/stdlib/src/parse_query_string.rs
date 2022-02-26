use super::url_util::{parse_query, query_inner_kind};

use url::form_urlencoded;
use vrl::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct ParseQueryString;

impl Function for ParseQueryString {
    fn identifier(&self) -> &'static str {
        "parse_query_string"
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            title: "parse query string",
            source: r#"parse_query_string("foo=1&bar=2")"#,
            result: Ok(r#"
                {
                    "foo": "1",
                    "bar": "2"
                }
            "#),
        }]
    }

    fn compile(
        &self,
        _state: &state::Compiler,
        _ctx: &FunctionCompileContext,
        mut arguments: ArgumentList,
    ) -> Compiled {
        let value = arguments.required("value");
        Ok(Box::new(ParseQueryStringFn { value }))
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[Parameter {
            keyword: "value",
            kind: kind::BYTES,
            required: true,
        }]
    }
}

#[derive(Debug, Clone)]
struct ParseQueryStringFn {
    value: Box<dyn Expression>,
}

impl Expression for ParseQueryStringFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        let bytes = self.value.resolve(ctx)?.try_bytes()?;

        let mut query_string = bytes.as_ref();
        if !query_string.is_empty() && query_string[0] == b'?' {
            query_string = &query_string[1..];
        }

        let query = form_urlencoded::parse(query_string);
        let result = parse_query(query);

        Ok(result.into())
    }

    fn type_def(&self, _: &state::Compiler) -> TypeDef {
        TypeDef::object(query_inner_kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    test_function![
        parse_query_string => ParseQueryString;

        complete {
            args: func_args![value: value!("foo=%2B1&bar=2&xyz=&abc")],
            want: Ok(value!({
                foo: "+1",
                bar: "2",
                xyz: "",
                abc: "",
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }

        multiple_values {
            args: func_args![value: value!("foo=bar&foo=xyz")],
            want: Ok(value!({
                foo: ["bar", "xyz"],
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }

        ruby_on_rails_multiple_values {
            args: func_args![value: value!("?foo%5b%5d=bar&foo%5b%5d=xyz")],
            want: Ok(value!({
                "foo[]": ["bar", "xyz"],
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }

        empty_key {
            args: func_args![value: value!("=&=")],
            want: Ok(value!({
                "": ["", ""],
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }

        single_key {
            args: func_args![value: value!("foo")],
            want: Ok(value!({
                foo: "",
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }

        empty {
            args: func_args![value: value!("")],
            want: Ok(value!({})),
            tdef: TypeDef::object(query_inner_kind()),
        }

        starts_with_question_mark {
            args: func_args![value: value!("?foo=bar")],
            want: Ok(value!({
                foo: "bar",
            })),
            tdef: TypeDef::object(query_inner_kind()),
        }
    ];
}
