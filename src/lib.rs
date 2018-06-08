#![cfg_attr(test, feature(test))]

#[macro_use]
extern crate combine;

use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum Ast {
    Lit(Value),
    Variable(String),
    Call(Box<Ast>, Vec<Ast>),
    Define(String, Box<Ast>),
}

#[derive(Clone)]
pub enum Value {
    Void,
    False,
    Int(u64),
    Function(Vec<String>, Vec<Ast>),
    InbuiltFunc(fn(Vec<Value>) -> Value),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;

        match (self, other) {
            (&Void, &Void) => true,
            (&False, &False) => true,
            (&Int(a), &Int(b)) => a == b,
            _ => false,
        }
    }
}

pub fn eval(program: Ast, variables: &mut HashMap<Rc<String>, Value>) -> Value {
    use self::Ast::*;
    use self::Value::*;

    match program {
        Lit(val) => val,
        Variable(name) => match variables.get(&name) {
            Some(v) => v.clone(),
            _ => panic!("Variable does not exist: {}", &name),
        },
        Call(func, arguments) => {
            let func = eval(*func, variables);

            match func {
                Function(args, body) => {
                    // Start a new scope, so all variables defined in the body of the
                    // function don't leak into the surrounding scope.
                    let mut new_scope = variables.clone();

                    if arguments.len() != args.len() {
                        println!("Called function with incorrect number of arguments (expected {}, got {})", args.len(), arguments.len());
                    }

                    for (name, val) in args.into_iter().zip(arguments) {
                        let val = eval(val, variables);
                        new_scope.insert(Rc::new(name), val);
                    }

                    let mut out = Void;

                    for stmt in body {
                        out = eval(stmt, &mut new_scope);
                    }

                    out
                }
                InbuiltFunc(func) => func(
                    arguments
                        .into_iter()
                        .map(|ast| eval(ast, variables))
                        .collect(),
                ),
                _ => panic!("Attempted to call a non-function"),
            }
        }
        Define(name, value) => {
            let value = eval(*value, variables);

            variables.insert(Rc::new(name), value);

            Void
        }
    }
}

parser! {
    pub fn expr[I]()(I) -> Ast where [I: combine::Stream<Item = char>] {
        use combine::parser::char::*;
        use combine::*;

        macro_rules! white {
            ($prs:expr) => {
                between(
                    skip_many(satisfy(char::is_whitespace)),
                    skip_many(satisfy(char::is_whitespace)),
                    $prs,
                )
            };
        }

        let lambda = char('\\');
        let eq = char('=');
        let flse = white!(string("#f")).map(|_| Ast::Lit(::Value::False));
        let ident = || white!(many1::<String, _>(letter()));
        let function = (
            white!(lambda),
            white!(between(char('('), char(')'), many::<Vec<_>, _>(ident()))),
            many::<Vec<_>, _>(expr()),
        ).map(|(_, a, b)| Ast::Lit(::Value::Function(a, b)));
        let define = (white!(eq), ident(), expr()).map(|(_, a, b)| Ast::Define(a, Box::new(b)));
        let lit_num = many1::<String, _>(digit())
            .map(|i| Ast::Lit(::Value::Int(i.parse().expect("Parsing integer failed"))));
        let call = (expr(), many(expr())).map(|(func, args)| Ast::Call(Box::new(func), args));

        white!(choice!(
            flse,
            lit_num,
            ident().map(Ast::Variable),
            between(char('('), char(')'), choice!(function, define, call))
        ))
    }
}

#[cfg(test)]
mod benches {
    extern crate test;

    use combine::Parser;

    use self::test::{black_box, Bencher};

    use super::{eval, expr, Value};
    use std::rc::Rc;

    // First we need some helper functions. These are used with the `InbuiltFunc`
    // constructor and act as native functions, similar to how you'd add functions
    // to the global namespace in Lua.
    //
    // This one simply sums the arguments.
    fn add(variables: Vec<Value>) -> Value {
        let mut out = 0u64;

        for v in variables {
            match v {
                Value::Int(i) => out += i,
                _ => println!("Tried to add a non-int"),
            }
        }

        Value::Int(out)
    }

    // This one checks the arguments for equality. I used `Void` to represent true
    // and `False` to represent false. This is mostly inspired by scheme, where
    // everything is true except for `#f`.
    fn eq(mut variables: Vec<Value>) -> Value {
        if let Some(last) = variables.pop() {
            for v in variables {
                if v != last {
                    return Value::False;
                }
            }

            Value::Void
        } else {
            Value::Void
        }
    }

    // This version of `if` doesn't lazily evaluate its branches, unlike every
    // other programming language in existence. To do lazy evaluation you make
    // the `then` and `else` branches return functions and then call the
    // functions.
    fn if_(variables: Vec<Value>) -> Value {
        let mut iter = variables.into_iter();
        let (first, second, third) = (
            iter.next().expect("No condition for if"),
            iter.next().expect("No body for if"),
            iter.next().unwrap_or(Value::Void),
        );
        assert!(iter.next().is_none(), "Too many arguments supplied to `if`");

        match first {
            Value::False => third,
            _ => second,
        }
    }

    // Here are our test program strings. Our language looks a lot like Lisp,
    // but it has the important distinction of being totally useless.
    //
    // This string is used to test the performance when programs include
    // deeply-nested structures. Nesting this deep is unlikely but it's a
    // good test for the parser's performance on nesting in general.
    const DEEP_NESTING: &str = "(((((((((((((((((((((((((((((((((((((((((((((test)))))))))))))))))))))))))))))))))))))))))))))";

    // This string is used to test the performance of when programs include
    // many variables of many different names, and many repetitions of the
    // same name. We'd expect real programs to contain lots of variables and
    // so it's important that we get good performance when parsing and
    // evaluating them.
    const MANY_VARIABLES: &str = r"
    ((\(a b c d e f g h i j k l m n o p q r s t u v w x y z)
      (a b c d e f g h i j k l m n o p q r s t u v w x y z)
      (b c d e f g h i j k l m n o p q r s t u v w x y z)
      (c d e f g h i j k l m n o p q r s t u v w x y z)
      (d e f g h i j k l m n o p q r s t u v w x y z)
      (e f g h i j k l m n o p q r s t u v w x y z)
      (f g h i j k l m n o p q r s t u v w x y z)
      (g h i j k l m n o p q r s t u v w x y z)
      (h i j k l m n o p q r s t u v w x y z)
      (i j k l m n o p q r s t u v w x y z)
      (j k l m n o p q r s t u v w x y z)
      (k l m n o p q r s t u v w x y z)
      (l m n o p q r s t u v w x y z)
      (m n o p q r s t u v w x y z)
      (n o p q r s t u v w x y z)
      (o p q r s t u v w x y z)
      (p q r s t u v w x y z)
      (q r s t u v w x y z)
      (r s t u v w x y z)
      (s t u v w x y z)
      (t u v w x y z)
      (u v w x y z)
      (v w x y z)
      (w x y z)
      (x y z)
      (y z)
      (z))
        ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore
        ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore ignore)
        ";

    // This is used to test that function calls aren't unnecessarily
    // expensive. It just passes the same value down and then back up
    // the stack.
    const NESTED_FUNC: &str = r"
    ((\(val)
      ((\(val)
        ((\(val)
          ((\(val)
            ((\(val)
              ((\(val)
                ((\(val)
                  ((\(val)
                    ((\(val)
                      ((\(val)
                        ((\(val)
                          val
                        ) val)
                      ) val)
                    ) val)
                  ) val)
                ) val)
              ) val)
            ) val)
          ) val)
        ) val)
      ) val)
    ) #f)
";

    // This is a more realistic program that uses every feature of
    // the language. It's not useful for finding hotspots but it's
    // definitely useful for seeing improvements.
    const REAL_CODE: &str = r"
(= increment (\(a)
  (add a 1)))
(= someval (increment 2))
(= double (\ (someval)
  (add someval someval)))
(= addfive (\ (first second third fourth fifth) (add first second third fourth fifth)))
(= second (\ (a a) a))
(= rec (\ (a)
  ((if (eq a 10)
       (\() 10)
       (\() (rec (add a 1)))))))
(= ne (\ (a b)
  (not (eq a b))))
(= not (\ (a)
  (if a #f)))

(double 5)
(addfive 1 2 3 4 5)
(second 1 2)
(rec 0)
(ne 1 2)
someval
";

    // Now we run the benchmarks. The parsing ones are very simple...
    #[bench]
    fn parse_deep_nesting(b: &mut Bencher) {
        b.iter(|| black_box(expr().easy_parse(DEEP_NESTING)))
    }

    #[bench]
    fn parse_many_variables(b: &mut Bencher) {
        b.iter(|| black_box(expr().easy_parse(MANY_VARIABLES)))
    }

    #[bench]
    fn parse_nested_func(b: &mut Bencher) {
        b.iter(|| black_box(expr().easy_parse(NESTED_FUNC)))
    }

    #[bench]
    fn parse_real_code(b: &mut Bencher) {
        b.iter(|| black_box(expr().easy_parse(REAL_CODE)))
    }

    // We only test parsing for this one. We could test the speed of
    // evaluating these expressions too but I personally prefer to
    // keep the benchmarks few and representative.
    #[bench]
    fn parse_literals(b: &mut Bencher) {
        let program_text = r"
            ((\()
               0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19
              20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39
              40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59
              50 51 52 53 54 55 56 57 58 59 60 61 62 63 64 65 66 67 68 69
              70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89
              90 91 92 93 94 95 96 97 98 99))
        ";

        b.iter(|| black_box(expr().easy_parse(program_text)))
    }

    // For the benchmarks that run the code we have to do a little more
    // work. We need to put some functions in the global namespace that
    // our testing code needs in order to run.
    #[bench]
    fn run_deep_nesting(b: &mut Bencher) {
        use std::collections::HashMap;

        // This just returns a function so `((whatever))` (equivalent
        // to `(whatever())()`) does something useful. Specifically
        // it just returns itself. We try to do as little work as
        // possible here so that our benchmark is still testing the
        // interpreter and not this function.
        fn callable(_: Vec<Value>) -> Value {
            Value::InbuiltFunc(callable)
        }

        let mut env = HashMap::new();
        env.insert(Rc::new("test".to_owned()), Value::InbuiltFunc(callable));

        let (program, _) = expr().easy_parse(DEEP_NESTING).unwrap();

        b.iter(|| black_box(eval(program.clone(), &mut env)));
    }

    #[bench]
    fn run_real_code(b: &mut Bencher) {
        use std::collections::HashMap;

        let mut env = HashMap::new();

        env.insert(Rc::new("eq".to_owned()), Value::InbuiltFunc(eq));
        env.insert(Rc::new("add".to_owned()), Value::InbuiltFunc(add));
        env.insert(Rc::new("if".to_owned()), Value::InbuiltFunc(if_));

        let (program, _) = ::combine::many1::<Vec<_>, _>(expr())
            .easy_parse(REAL_CODE)
            .unwrap();

        b.iter(|| {
            let mut env = env.clone();
            for line in &program {
                black_box(eval(line.clone(), &mut env));
            }
        });
    }

    #[bench]
    fn run_many_variables(b: &mut Bencher) {
        use std::collections::HashMap;

        // This just takes anything and returns `Void`. We just
        // want a function that can take any number of arguments
        // but we don't want that function to do anything useful
        // since, again, the benchmark should be of the
        // interpreter's code.
        fn ignore(_: Vec<Value>) -> Value {
            Value::Void
        }

        let (program, _) = expr().easy_parse(MANY_VARIABLES).unwrap();

        let mut env = HashMap::new();

        env.insert(Rc::new("ignore".to_owned()), Value::InbuiltFunc(ignore));

        b.iter(|| black_box(eval(program.clone(), &mut env)));
    }

    #[bench]
    fn run_nested_func(b: &mut Bencher) {
        use std::collections::HashMap;

        let (program, _) = expr().easy_parse(NESTED_FUNC).unwrap();
        let mut env = HashMap::new();
        b.iter(|| black_box(eval(program.clone(), &mut env)));
    }
}
