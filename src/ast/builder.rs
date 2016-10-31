//! Defines an interfaces to receive parse data and construct ASTs.
//!
//! This allows the parser to remain agnostic of the required source
//! representation, and frees up the library user to substitute their own.
//! If one does not require a custom AST representation, this module offers
//! a reasonable default builder implementation.
//!
//! If a custom AST representation is required you will need to implement
//! the `Builder` trait for your AST. Otherwise you can provide the `DefaultBuilder`
//! struct to the parser if you wish to use the default AST implementation.

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use ast::{self, AndOr, AndOrList, Arithmetic, Command, CompoundCommand,
          CompoundCommandKind, ComplexWord, DefaultPipeableCommand, ListableCommand, Parameter,
          ParameterSubstitution, PipeableCommand, Redirect, SimpleCommand, SimpleWord,
          TopLevelCommand, TopLevelWord, Word};
use parse::ParseResult;
use void::Void;

/// An indicator to the builder of how complete commands are separated.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SeparatorKind {
    /// A semicolon appears between commands, normally indicating a sequence.
    Semi,
    /// An ampersand appears between commands, normally indicating an asyncronous job.
    Amp,
    /// A newline (and possibly a comment) appears at the end of a command before the next.
    Newline,
    /// The command was delimited by a token (e.g. a compound command delimiter) or
    /// the end of input, but is *not* followed by another sequential command.
    Other,
}

/// An indicator to the builder whether a `while` or `until` command was parsed.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum LoopKind {
    /// A `while` command was parsed, normally indicating the loop's body should be run
    /// while the guard's exit status is successful.
    While,
    /// An `until` command was parsed, normally indicating the loop's body should be run
    /// until the guard's exit status becomes successful.
    Until,
}

/// A grouping of a list of commands and any comments trailing after the commands.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CommandGroup<C> {
    /// The sequential list of commands.
    pub commands: Vec<C>,
    /// Any trailing comments appearing on the next line after the last command.
    pub trailing_comments: Vec<Newline>,
}

/// A grouping of guard and body commands, and any comments they may have.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GuardBodyPairGroup<C> {
    /// The guard commands, which if successful, should lead to the
    /// execution of the body commands.
    pub guard: CommandGroup<C>,
    /// The body commands to execute if the guard is successful.
    pub body: CommandGroup<C>,
}

/// Parsed fragments relating to a shell `if` command.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct IfFragments<C> {
    /// A list of conditionals branches.
    pub conditionals: Vec<GuardBodyPairGroup<C>>,
    /// The `else` branch, if any,
    pub else_branch: Option<CommandGroup<C>>,
}

/// Parsed fragments relating to a shell `for` command.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ForFragments<W, C> {
    /// The name of the variable to which each of the words will be bound.
    pub var: String,
    /// A comment that begins on the same line as the variable declaration.
    pub var_comment: Option<Newline>,
    /// Any comments after the variable declaration, a group of words to
    /// iterator over, and comment defined on the same line as the words.
    pub words: Option<(Vec<Newline>, Vec<W>, Option<Newline>)>,
    /// Any comments that appear after the `words` declaration (if it exists),
    /// but before the body of commands.
    pub pre_body_comments: Vec<Newline>,
    /// The body to be invoked for every iteration.
    pub body: CommandGroup<C>,
}

/// Parsed fragments relating to a shell `case` command.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CaseFragments<W, C> {
    /// The word to be matched against.
    pub word: W,
    /// The comments appearing after the word to match but before the `in` reserved word.
    pub post_word_comments: Vec<Newline>,
    /// A comment appearing immediately after the `in` reserved word,
    /// yet still on the same line.
    pub in_comment: Option<Newline>,
    /// All the possible branches of the `case` command.
    pub arms: Vec<CaseArm<W, C>>,
    /// The comments appearing after the last arm but before the `esac` reserved word.
    pub post_arms_comments: Vec<Newline>,
}

/// An individual "unit of execution" within a `case` command.
///
/// Each arm has a number of pattern alternatives, and a body
/// of commands to run if any pattern matches.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CaseArm<W, C> {
    /// The patterns which correspond to this case arm.
    pub patterns: CasePatternFragments<W>,
    /// The body of commands to run if any pattern matches.
    pub body: CommandGroup<C>,
    /// A comment appearing at the end of the arm declaration,
    /// i.e. after `;;` but on the same line.
    pub arm_comment: Option<Newline>,
}

/// Parsed fragments relating to patterns in a shell `case` command.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CasePatternFragments<W> {
    /// Comments appearing after a previous arm, but before the start of a pattern.
    pub pre_pattern_comments: Vec<Newline>,
    /// Pattern alternatives which all correspond to the same case arm.
    pub pattern_alternatives: Vec<W>,
    /// A comment appearing at the end of the pattern declaration on the same line.
    pub pattern_comment: Option<Newline>,
}

/// An indicator to the builder what kind of complex word was parsed.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ComplexWordKind<C> {
    /// Several distinct words concatenated together.
    Concat(Vec<WordKind<C>>),
    /// A regular word.
    Single(WordKind<C>),
}

/// An indicator to the builder what kind of word was parsed.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum WordKind<C> {
    /// A regular word.
    Simple(SimpleWordKind<C>),
    /// List of words concatenated within double quotes.
    DoubleQuoted(Vec<SimpleWordKind<C>>),
    /// List of words concatenated within single quotes. Virtually
    /// identical as a literal, but makes a distinction between the two.
    SingleQuoted(String),
}

/// An indicator to the builder what kind of simple word was parsed.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SimpleWordKind<C> {
    /// A non-special literal word.
    Literal(String),
    /// Access of a value inside a parameter, e.g. `$foo` or `$$`.
    Param(Parameter),
    /// A parameter substitution, e.g. `${param-word}`.
    Subst(Box<ParameterSubstitutionKind<ComplexWordKind<C>, C>>),
    /// Represents the standard output of some command, e.g. \`echo foo\`.
    CommandSubst(CommandGroup<C>),
    /// A token which normally has a special meaning is treated as a literal
    /// because it was escaped, typically with a backslash, e.g. `\"`.
    Escaped(String),
    /// Represents `*`, useful for handling pattern expansions.
    Star,
    /// Represents `?`, useful for handling pattern expansions.
    Question,
    /// Represents `[`, useful for handling pattern expansions.
    SquareOpen,
    /// Represents `]`, useful for handling pattern expansions.
    SquareClose,
    /// Represents `~`, useful for handling tilde expansions.
    Tilde,
    /// Represents `:`, useful for handling tilde expansions.
    Colon,
}

/// Represents redirecting a command's file descriptors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RedirectKind<W> {
    /// Open a file for reading, e.g. `[n]< file`.
    Read(Option<u16>, W),
    /// Open a file for writing after truncating, e.g. `[n]> file`.
    Write(Option<u16>, W),
    /// Open a file for reading and writing, e.g. `[n]<> file`.
    ReadWrite(Option<u16>, W),
    /// Open a file for writing, appending to the end, e.g. `[n]>> file`.
    Append(Option<u16>, W),
    /// Open a file for writing, failing if the `noclobber` shell option is set, e.g. `[n]>| file`.
    Clobber(Option<u16>, W),
    /// Lines contained in the source that should be provided by as input to a file descriptor.
    Heredoc(Option<u16>, W),
    /// Duplicate a file descriptor for reading, e.g. `[n]<& [n|-]`.
    DupRead(Option<u16>, W),
    /// Duplicate a file descriptor for writing, e.g. `[n]>& [n|-]`.
    DupWrite(Option<u16>, W),
}

/// Represents the type of parameter that was parsed
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ParameterSubstitutionKind<W, C> {
    /// Returns the standard output of running a command, e.g. `$(cmd)`
    Command(CommandGroup<C>),
    /// Returns the length of the value of a parameter, e.g. ${#param}
    Len(Parameter),
    /// Returns the resulting value of an arithmetic subsitution, e.g. `$(( x++ ))`
    Arith(Option<Arithmetic>),
    /// Use a provided value if the parameter is null or unset, e.g.
    /// `${param:-[word]}`.
    /// The boolean indicates the presence of a `:`, and that if the parameter has
    /// a null value, that situation should be treated as if the parameter is unset.
    Default(bool, Parameter, Option<W>),
    /// Assign a provided value to the parameter if it is null or unset,
    /// e.g. `${param:=[word]}`.
    /// The boolean indicates the presence of a `:`, and that if the parameter has
    /// a null value, that situation should be treated as if the parameter is unset.
    Assign(bool, Parameter, Option<W>),
    /// If the parameter is null or unset, an error should result with the provided
    /// message, e.g. `${param:?[word]}`.
    /// The boolean indicates the presence of a `:`, and that if the parameter has
    /// a null value, that situation should be treated as if the parameter is unset.
    Error(bool, Parameter, Option<W>),
    /// If the parameter is NOT null or unset, a provided word will be used,
    /// e.g. `${param:+[word]}`.
    /// The boolean indicates the presence of a `:`, and that if the parameter has
    /// a null value, that situation should be treated as if the parameter is unset.
    Alternative(bool, Parameter, Option<W>),
    /// Remove smallest suffix pattern, e.g. `${param%pattern}`
    RemoveSmallestSuffix(Parameter, Option<W>),
    /// Remove largest suffix pattern, e.g. `${param%%pattern}`
    RemoveLargestSuffix(Parameter, Option<W>),
    /// Remove smallest prefix pattern, e.g. `${param#pattern}`
    RemoveSmallestPrefix(Parameter, Option<W>),
    /// Remove largest prefix pattern, e.g. `${param##pattern}`
    RemoveLargestPrefix(Parameter, Option<W>),
}

/// Represents a parsed newline, more specifically, the presense of a comment
/// immediately preceeding the newline.
///
/// Since shell comments are usually treated as a newline, they can be present
/// anywhere a newline can be as well. Thus if it is desired to retain comments
/// they can be optionally attached to a parsed newline.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Newline(pub Option<String>);

/// A trait which defines an interface which the parser defined in the `parse` module
/// uses to delegate Abstract Syntax Tree creation. The methods defined here correspond
/// to their respectively named methods on the parser, and accept the relevant data for
/// each shell command type.
pub trait Builder {
    /// The type which represents a complete, top-level command.
    type Command;
    /// The type which represents an and/or list of commands.
    type CommandList;
    /// The type which represents a command that can be used in an and/or command list.
    type ListableCommand;
    /// The type which represents a command that can be used in a pipeline.
    type PipeableCommand;
    /// The type which represents compound commands like `if`, `case`, `for`, etc.
    type CompoundCommand;
    /// The type which represents shell words, which can be command names or arguments.
    type Word;
    /// The type which represents a file descriptor redirection.
    type Redirect;
    /// A type for returning custom parse/build errors.
    type Error;

    /// Invoked once a complete command is found. That is, a command delimited by a
    /// newline, semicolon, ampersand, or the end of input.
    ///
    /// # Arguments
    /// * pre_cmd_comments: any comments that appear before the start of the command
    /// * list: an and/or list of commands previously generated by the same builder
    /// * separator: indicates how the command was delimited
    /// * cmd_comment: a comment that appears at the end of the command
    fn complete_command(&mut self,
                        pre_cmd_comments: Vec<Newline>,
                        list: Self::CommandList,
                        separator: SeparatorKind,
                        cmd_comment: Option<Newline>)
        -> ParseResult<Self::Command, Self::Error>;

    /// Invoked when multiple commands are parsed which are separated by `&&` or `||`.
    /// Typically after the first command is run, each of the following commands may or
    /// may not be executed, depending on the exit status of the previously executed command.
    ///
    /// # Arguments
    /// * first: the first command before any `&&` or `||` separator
    /// * rest: A collection of comments after the last separator and the next command.
    fn and_or_list(&mut self,
              first: Self::ListableCommand,
              rest: Vec<(Vec<Newline>, AndOr<Self::ListableCommand>)>)
        -> ParseResult<Self::CommandList, Self::Error>;

    /// Invoked when a pipeline of commands is parsed.
    /// A pipeline is one or more commands where the standard output of the previous
    /// typically becomes the standard input of the next.
    ///
    /// # Arguments
    /// * bang: the presence of a `!` at the start of the pipeline, typically indicating
    /// that the pipeline's exit status should be logically inverted.
    /// * cmds: a collection of tuples which are any comments appearing after a pipe token, followed
    /// by the command itself, all in the order they were parsed
    fn pipeline(&mut self,
                bang: bool,
                cmds: Vec<(Vec<Newline>, Self::PipeableCommand)>)
        -> ParseResult<Self::ListableCommand, Self::Error>;

    /// Invoked when the "simplest" possible command is parsed: an executable with arguments.
    ///
    /// # Arguments
    /// * env_vars: environment variables to be defined only for the command before it is run.
    /// * cmd: a tuple of the name of the command to be run and any arguments. This value is
    /// optional since the shell grammar permits that a simple command be made up of only env
    /// var definitions or redirects (or both).
    /// * redirects: redirection of any file descriptors to/from other file descriptors or files.
    fn simple_command(&mut self,
                      env_vars: Vec<(String, Option<Self::Word>)>,
                      cmd: Option<(Self::Word, Vec<Self::Word>)>,
                      redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::PipeableCommand, Self::Error>;

    /// Invoked when a non-zero number of commands were parsed between balanced curly braces.
    /// Typically these commands should run within the current shell environment.
    ///
    /// # Arguments
    /// * cmds: the commands that were parsed between braces
    /// * redirects: any redirects to be applied over the **entire** group of commands
    fn brace_group(&mut self,
                   cmds: CommandGroup<Self::Command>,
                   redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Invoked when a non-zero number of commands were parsed between balanced parentheses.
    /// Typically these commands should run within their own environment without affecting
    /// the shell's global environment.
    ///
    /// # Arguments
    /// * cmds: the commands that were parsed between parens
    /// * redirects: any redirects to be applied over the **entire** group of commands
    fn subshell(&mut self,
                cmds: CommandGroup<Self::Command>,
                redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Invoked when a loop command like `while` or `until` is parsed.
    /// Typically these commands will execute their body based on the exit status of their guard.
    ///
    /// # Arguments
    /// * kind: the type of the loop: `while` or `until`
    /// * guard: commands that determine how long the loop will run for
    /// * body: commands to be run every iteration of the loop
    /// * redirects: any redirects to be applied over **all** commands part of the loop
    fn loop_command(&mut self,
                    kind: LoopKind,
                    guard_body_pair: GuardBodyPairGroup<Self::Command>,
                    redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Invoked when an `if` conditional command is parsed.
    /// Typically an `if` command is made up of one or more guard-body pairs, where the body
    /// of the first successful corresponding guard is executed. There can also be an optional
    /// `else` part to be run if no guard is successful.
    ///
    /// # Arguments
    /// * fragments: parsed fragments relating to a shell `if` command.
    /// * redirects: any redirects to be applied over **all** commands within the `if` command
    fn if_command(&mut self,
                  fragments: IfFragments<Self::Command>,
                  redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Invoked when a `for` command is parsed.
    /// Typically a `for` command binds a variable to each member in a group of words and
    /// invokes its body with that variable present in the environment. If no words are
    /// specified, the command will iterate over the arguments to the script or enclosing function.
    ///
    /// # Arguments
    /// * fragments: parsed fragments relating to a shell `for` command.
    /// * redirects: any redirects to be applied over **all** commands within the `for` command
    fn for_command(&mut self,
                   fragments: ForFragments<Self::Word, Self::Command>,
                   redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Invoked when a `case` command is parsed.
    /// Typically this command will execute certain commands when a given word matches a pattern.
    ///
    /// # Arguments
    /// * fragments: parsed fragments relating to a shell `case` command.
    /// * redirects: any redirects to be applied over **all** commands part of the `case` block
    fn case_command(&mut self,
                    fragments: CaseFragments<Self::Word, Self::Command>,
                    redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>;

    /// Bridges the gap between a `PipeableCommand` and a `CompoundCommand` since
    /// `CompoundCommand`s are typically `PipeableCommand`s as well.
    ///
    /// # Arguments
    /// cmd: The `CompoundCommand` to convert into a `PipeableCommand`
    fn compound_command_as_pipeable(&mut self,
                                    cmd: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>;

    /// Invoked when a function declaration is parsed.
    /// Typically a function declaration overwrites any previously defined function
    /// within the current environment.
    ///
    /// # Arguments
    /// * name: the name of the function to be created
    /// * post_name_comments: any comments appearing after the function name but before the body
    /// * body: commands to be run when the function is invoked
    fn function_declaration(&mut self,
                            name: String,
                            post_name_comments: Vec<Newline>,
                            body: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>;

    /// Invoked when only comments are parsed with no commands following.
    /// This can occur if an entire shell script is commented out or if there
    /// are comments present at the end of the script.
    ///
    /// # Arguments
    /// * comments: the parsed comments
    fn comments(&mut self,
                comments: Vec<Newline>)
        -> ParseResult<(), Self::Error>;

    /// Invoked when a word is parsed.
    ///
    /// # Arguments
    /// * kind: the type of word that was parsed
    fn word(&mut self,
            kind: ComplexWordKind<Self::Command>)
        -> ParseResult<Self::Word, Self::Error>;

    /// Invoked when a redirect is parsed.
    ///
    /// # Arguments
    /// * kind: the type of redirect that was parsed
    fn redirect(&mut self,
                kind: RedirectKind<Self::Word>)
        -> ParseResult<Self::Redirect, Self::Error>;
}

/// A `Builder` implementation which builds shell commands using the AST definitions in the `ast` module.
#[derive(Debug, Copy, Clone)]
pub struct DefaultBuilder<T>(PhantomData<T>);

/// A `DefaultBuilder` implementation which uses regular `String`s when
/// representing shell words.
pub type StringBuilder = DefaultBuilder<String>;

/// A `DefaultBuilder` implementation which uses `Rc<String>`s when
/// representing shell words.
pub type RcBuilder = DefaultBuilder<Rc<String>>;

/// A `DefaultBuilder` implementation which uses `Arc<String>`s when
/// representing shell words.
pub type ArcBuilder = DefaultBuilder<Arc<String>>;

impl<T> ::std::default::Default for DefaultBuilder<T> {
    fn default() -> Self {
        DefaultBuilder::new()
    }
}

impl<T> DefaultBuilder<T> {
    /// Constructs a builder.
    pub fn new() -> Self {
        DefaultBuilder(PhantomData)
    }
}

impl<T: From<String>> Builder for DefaultBuilder<T> {
    type Command         = TopLevelCommand<T>;
    type CommandList     = AndOrList<Self::ListableCommand>;
    type ListableCommand = ListableCommand<Self::PipeableCommand>;
    type PipeableCommand = DefaultPipeableCommand<T, Self::Word, Self::Command>;
    type CompoundCommand = CompoundCommand<CompoundCommandKind<Self::Word, Self::Command>, Self::Redirect>;
    type Word            = TopLevelWord<T>;
    type Redirect        = Redirect<Self::Word>;
    type Error           = Void;

    /// Constructs a `Command::Job` node with the provided inputs if the command
    /// was delimited by an ampersand or the command itself otherwise.
    fn complete_command(&mut self,
                        _pre_cmd_comments: Vec<Newline>,
                        list: Self::CommandList,
                        separator: SeparatorKind,
                        _cmd_comment: Option<Newline>)
        -> ParseResult<Self::Command, Self::Error>
    {
        let cmd = match separator {
            SeparatorKind::Semi  |
            SeparatorKind::Other |
            SeparatorKind::Newline => Command::List(list),
            SeparatorKind::Amp => Command::Job(list),
        };

        Ok(TopLevelCommand(cmd))
    }

    /// Constructs a `Command::List` node with the provided inputs.
    fn and_or_list(&mut self,
              first: Self::ListableCommand,
              rest: Vec<(Vec<Newline>, AndOr<Self::ListableCommand>)>)
        -> ParseResult<Self::CommandList, Self::Error>
    {
        Ok(AndOrList {
            first: first,
            rest: rest.into_iter().map(|(_, c)| c).collect(),
        })
    }

    /// Constructs a `Command::Pipe` node with the provided inputs or a `Command::Simple`
    /// node if only a single command with no status inversion is supplied.
    fn pipeline(&mut self,
                bang: bool,
                cmds: Vec<(Vec<Newline>, Self::PipeableCommand)>)
        -> ParseResult<Self::ListableCommand, Self::Error>
    {
        debug_assert_eq!(cmds.is_empty(), false);
        let mut cmds: Vec<_> = cmds.into_iter().map(|(_, c)| c).collect();

        // Pipe is the only AST node which allows for a status
        // negation, so we are forced to use it even if we have a single
        // command. Otherwise there is no need to wrap it further.
        if bang || cmds.len() > 1 {
            cmds.shrink_to_fit();
            Ok(ListableCommand::Pipe(bang, cmds))
        } else {
            Ok(ListableCommand::Single(cmds.pop().unwrap()))
        }
    }

    /// Constructs a `Command::Simple` node with the provided inputs.
    fn simple_command(&mut self,
                      env_vars: Vec<(String, Option<Self::Word>)>,
                      mut cmd: Option<(Self::Word, Vec<Self::Word>)>,
                      mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        redirects.shrink_to_fit();

        if let Some(&mut (_, ref mut args)) = cmd.as_mut() {
            args.shrink_to_fit();
        }

        Ok(PipeableCommand::Simple(Box::new(SimpleCommand {
            cmd: cmd,
            vars: env_vars.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            io: redirects,
        })))
    }

    /// Constructs a `CompoundCommand::Brace` node with the provided inputs.
    fn brace_group(&mut self,
                   cmd_group: CommandGroup<Self::Command>,
                   mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        let mut cmds = cmd_group.commands;
        cmds.shrink_to_fit();
        redirects.shrink_to_fit();
        Ok(CompoundCommand {
            kind: CompoundCommandKind::Brace(cmds),
            io: redirects,
        })
    }

    /// Constructs a `CompoundCommand::Subshell` node with the provided inputs.
    fn subshell(&mut self,
                cmd_group: CommandGroup<Self::Command>,
                mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        let mut cmds = cmd_group.commands;
        cmds.shrink_to_fit();
        redirects.shrink_to_fit();
        Ok(CompoundCommand {
            kind: CompoundCommandKind::Subshell(cmds),
            io: redirects,
        })
    }

    /// Constructs a `CompoundCommand::Loop` node with the provided inputs.
    fn loop_command(&mut self,
                    kind: LoopKind,
                    guard_body_pair: GuardBodyPairGroup<Self::Command>,
                    mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        let mut guard = guard_body_pair.guard.commands;
        let mut body = guard_body_pair.body.commands;

        guard.shrink_to_fit();
        body.shrink_to_fit();
        redirects.shrink_to_fit();

        let guard_body_pair = ast::GuardBodyPair {
            guard: guard,
            body: body,
        };

        let loop_cmd = match kind {
            LoopKind::While => CompoundCommandKind::While(guard_body_pair),
            LoopKind::Until => CompoundCommandKind::Until(guard_body_pair),
        };

        Ok(CompoundCommand {
            kind: loop_cmd,
            io: redirects,
        })
    }

    /// Constructs a `CompoundCommand::If` node with the provided inputs.
    fn if_command(&mut self,
                  fragments: IfFragments<Self::Command>,
                  mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        let IfFragments { conditionals, else_branch } = fragments;

        let conditionals = conditionals.into_iter()
            .map(|gbp| {
                let mut guard = gbp.guard.commands;
                let mut body = gbp.body.commands;

                guard.shrink_to_fit();
                body.shrink_to_fit();

                ast::GuardBodyPair {
                    guard: guard,
                    body: body,
                }
            })
            .collect();

        let else_branch = else_branch.map(|CommandGroup { commands: mut els, .. }| {
            els.shrink_to_fit();
            els
        });

        redirects.shrink_to_fit();

        Ok(CompoundCommand {
            kind: CompoundCommandKind::If {
                conditionals: conditionals,
                else_branch: else_branch,
            },
            io: redirects,
        })
    }

    /// Constructs a `CompoundCommand::For` node with the provided inputs.
    fn for_command(&mut self,
                   mut fragments: ForFragments<Self::Word, Self::Command>,
                   mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        let words = fragments.words.map(|(_, mut words, _)| {
            words.shrink_to_fit();
            words
        });

        let mut body = fragments.body.commands;
        body.shrink_to_fit();

        fragments.var.shrink_to_fit();
        redirects.shrink_to_fit();

        Ok(CompoundCommand {
            kind: CompoundCommandKind::For {
                var: fragments.var,
                words: words,
                body: body,
            },
            io: redirects
        })
    }

    /// Constructs a `CompoundCommand::Case` node with the provided inputs.
    fn case_command(&mut self,
                    fragments: CaseFragments<Self::Word, Self::Command>,
                    mut redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        use ast::PatternBodyPair;

        let arms = fragments.arms.into_iter().map(|arm| {
            let mut patterns = arm.patterns.pattern_alternatives;
            patterns.shrink_to_fit();

            let mut body = arm.body.commands;
            body.shrink_to_fit();

            PatternBodyPair {
                patterns: patterns,
                body: body,
            }
        }).collect();

        redirects.shrink_to_fit();
        Ok(CompoundCommand {
            kind: CompoundCommandKind::Case {
                word: fragments.word,
                arms: arms,
            },
            io: redirects,
        })
    }

    /// Converts a `CompoundCommand` into a `PipeableCommand`.
    fn compound_command_as_pipeable(&mut self,
                                    cmd: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        Ok(PipeableCommand::Compound(Box::new(cmd)))
    }

    /// Constructs a `Command::FunctionDef` node with the provided inputs.
    fn function_declaration(&mut self,
                            name: String,
                            _post_name_comments: Vec<Newline>,
                            body: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        Ok(PipeableCommand::FunctionDef(name.into(), Rc::new(body)))
    }

    /// Ignored by the builder.
    fn comments(&mut self, _comments: Vec<Newline>) -> ParseResult<(), Self::Error> {
        Ok(())
    }

    /// Constructs a `ast::Word` from the provided input.
    fn word(&mut self, kind: ComplexWordKind<Self::Command>) -> ParseResult<Self::Word, Self::Error> {
        use self::ParameterSubstitutionKind::*;

        macro_rules! map {
            ($pat:expr) => {
                match $pat {
                    Some(w) => Some(try!(self.word(w))),
                    None => None,
                }
            }
        }

        fn map_arith<T: From<String>>(kind: Arithmetic) -> Arithmetic<T> {
            use ast::Arithmetic::*;
            match kind {
                Var(v)           => Var(v.into()),
                Literal(l)       => Literal(l.into()),
                Pow(a, b)        => Pow(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                PostIncr(p)      => PostIncr(p.into()),
                PostDecr(p)      => PostDecr(p.into()),
                PreIncr(p)       => PreIncr(p.into()),
                PreDecr(p)       => PreDecr(p.into()),
                UnaryPlus(a)     => UnaryPlus(Box::new(map_arith(*a))),
                UnaryMinus(a)    => UnaryMinus(Box::new(map_arith(*a))),
                LogicalNot(a)    => LogicalNot(Box::new(map_arith(*a))),
                BitwiseNot(a)    => BitwiseNot(Box::new(map_arith(*a))),
                Mult(a, b)       => Mult(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Div(a, b)        => Div(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Modulo(a, b)     => Modulo(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Add(a, b)        => Add(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Sub(a, b)        => Sub(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                ShiftLeft(a, b)  => ShiftLeft(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                ShiftRight(a, b) => ShiftRight(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Less(a, b)       => Less(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                LessEq(a, b)     => LessEq(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Great(a, b)      => Great(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                GreatEq(a, b)    => GreatEq(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Eq(a, b)         => Eq(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                NotEq(a, b)      => NotEq(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                BitwiseAnd(a, b) => BitwiseAnd(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                BitwiseXor(a, b) => BitwiseXor(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                BitwiseOr(a, b)  => BitwiseOr(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                LogicalAnd(a, b) => LogicalAnd(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                LogicalOr(a, b)  => LogicalOr(Box::new(map_arith(*a)), Box::new(map_arith(*b))),
                Ternary(a, b, c) =>
                    Ternary(Box::new(map_arith(*a)), Box::new(map_arith(*b)), Box::new(map_arith(*c))),
                Assign(v, a) => Assign(v.into(), Box::new(map_arith(*a))),
                Sequence(ariths) => Sequence(ariths.into_iter().map(map_arith).collect()),
            }
        }

        let map_param = |kind: Parameter| -> Parameter<T> {
            use ast::Parameter::*;
            match kind {
                At            => At,
                Star          => Star,
                Pound         => Pound,
                Question      => Question,
                Dash          => Dash,
                Dollar        => Dollar,
                Bang          => Bang,
                Positional(p) => Positional(p),
                Var(v)        => Var(v.into()),
            }
        };

        let mut map_simple = |kind| {
            let simple = match kind {
                SimpleWordKind::Literal(s)      => SimpleWord::Literal(s.into()),
                SimpleWordKind::Escaped(s)      => SimpleWord::Escaped(s.into()),
                SimpleWordKind::Param(p)        => SimpleWord::Param(map_param(p)),
                SimpleWordKind::Star            => SimpleWord::Star,
                SimpleWordKind::Question        => SimpleWord::Question,
                SimpleWordKind::SquareOpen      => SimpleWord::SquareOpen,
                SimpleWordKind::SquareClose     => SimpleWord::SquareClose,
                SimpleWordKind::Tilde           => SimpleWord::Tilde,
                SimpleWordKind::Colon           => SimpleWord::Colon,

                SimpleWordKind::CommandSubst(c) => SimpleWord::Subst(
                    Box::new(ParameterSubstitution::Command(c.commands))
                ),

                SimpleWordKind::Subst(s) => {
                    // Force a move out of the boxed substitution. For some reason doing
                    // the deref in the match statment gives a strange borrow failure
                    let s = *s;
                    let subst = match s {
                        Len(p) => ParameterSubstitution::Len(map_param(p)),
                        Command(c) => ParameterSubstitution::Command(c.commands),
                        Arith(a) => ParameterSubstitution::Arith(a.map(map_arith)),
                        Default(c, p, w) =>
                            ParameterSubstitution::Default(c, map_param(p), map!(w)),
                        Assign(c, p, w) =>
                            ParameterSubstitution::Assign(c, map_param(p), map!(w)),
                        Error(c, p, w) =>
                            ParameterSubstitution::Error(c, map_param(p), map!(w)),
                        Alternative(c, p, w) =>
                            ParameterSubstitution::Alternative(c, map_param(p), map!(w)),
                        RemoveSmallestSuffix(p, w) =>
                            ParameterSubstitution::RemoveSmallestSuffix(map_param(p), map!(w)),
                        RemoveLargestSuffix(p, w)  =>
                            ParameterSubstitution::RemoveLargestSuffix(map_param(p), map!(w)),
                        RemoveSmallestPrefix(p, w) =>
                            ParameterSubstitution::RemoveSmallestPrefix(map_param(p), map!(w)),
                        RemoveLargestPrefix(p, w)  =>
                            ParameterSubstitution::RemoveLargestPrefix(map_param(p), map!(w)),
                    };
                    SimpleWord::Subst(Box::new(subst))
                },
            };
            Ok(simple)
        };

        let mut map_word = |kind| {
            let word = match kind {
                WordKind::Simple(s)       => Word::Simple(try!(map_simple(s))),
                WordKind::SingleQuoted(s) => Word::SingleQuoted(s.into()),
                WordKind::DoubleQuoted(v) => Word::DoubleQuoted(try!(
                    v.into_iter()
                     .map(&mut map_simple)
                     .collect::<ParseResult<Vec<_>, _>>()
                )),
            };
            Ok(word)
        };

        let word = match compress(kind) {
            ComplexWordKind::Single(s)     => ComplexWord::Single(try!(map_word(s))),
            ComplexWordKind::Concat(words) => ComplexWord::Concat(try!(
                    words.into_iter()
                         .map(map_word)
                         .collect::<ParseResult<Vec<_>, _>>()
            )),
        };

        Ok(TopLevelWord(word))
    }

    /// Constructs a `ast::Redirect` from the provided input.
    fn redirect(&mut self,
                kind: RedirectKind<Self::Word>)
        -> ParseResult<Self::Redirect, Self::Error>
    {
        let io = match kind {
            RedirectKind::Read(fd, path)      => Redirect::Read(fd, path),
            RedirectKind::Write(fd, path)     => Redirect::Write(fd, path),
            RedirectKind::ReadWrite(fd, path) => Redirect::ReadWrite(fd, path),
            RedirectKind::Append(fd, path)    => Redirect::Append(fd, path),
            RedirectKind::Clobber(fd, path)   => Redirect::Clobber(fd, path),
            RedirectKind::Heredoc(fd, body)   => Redirect::Heredoc(fd, body),
            RedirectKind::DupRead(src, dst)   => Redirect::DupRead(src, dst),
            RedirectKind::DupWrite(src, dst)  => Redirect::DupWrite(src, dst),
        };

        Ok(io)
    }
}

impl<'a, T: Builder + ?Sized> Builder for &'a mut T {
    type Command         = T::Command;
    type CommandList     = T::CommandList;
    type ListableCommand = T::ListableCommand;
    type PipeableCommand = T::PipeableCommand;
    type CompoundCommand = T::CompoundCommand;
    type Word            = T::Word;
    type Redirect        = T::Redirect;
    type Error           = T::Error;

    fn complete_command(&mut self,
                        pre_cmd_comments: Vec<Newline>,
                        list: Self::CommandList,
                        separator: SeparatorKind,
                        cmd_comment: Option<Newline>)
        -> ParseResult<Self::Command, Self::Error>
    {
        (**self).complete_command(pre_cmd_comments, list, separator, cmd_comment)
    }

    fn and_or_list(&mut self,
              first: Self::ListableCommand,
              rest: Vec<(Vec<Newline>, AndOr<Self::ListableCommand>)>)
        -> ParseResult<Self::CommandList, Self::Error>
    {
        (**self).and_or_list(first, rest)
    }

    fn pipeline(&mut self,
                bang: bool,
                cmds: Vec<(Vec<Newline>, Self::PipeableCommand)>)
        -> ParseResult<Self::ListableCommand, Self::Error>
    {
        (**self).pipeline(bang, cmds)
    }

    fn simple_command(&mut self,
                      env_vars: Vec<(String, Option<Self::Word>)>,
                      cmd: Option<(Self::Word, Vec<Self::Word>)>,
                      redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        (**self).simple_command(env_vars, cmd, redirects)
    }

    fn brace_group(&mut self,
                   cmds: CommandGroup<Self::Command>,
                   redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).brace_group(cmds, redirects)
    }

    fn subshell(&mut self,
                cmds: CommandGroup<Self::Command>,
                redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).subshell(cmds, redirects)
    }

    fn loop_command(&mut self,
                    kind: LoopKind,
                    guard_body_pair: GuardBodyPairGroup<Self::Command>,
                    redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).loop_command(kind, guard_body_pair, redirects)
    }

    fn if_command(&mut self,
                  fragments: IfFragments<Self::Command>,
                  redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).if_command(fragments, redirects)
    }

    fn for_command(&mut self,
                   fragments: ForFragments<Self::Word, Self::Command>,
                   redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).for_command(fragments, redirects)
    }

    fn case_command(&mut self,
                    fragments: CaseFragments<Self::Word, Self::Command>,
                    redirects: Vec<Self::Redirect>)
        -> ParseResult<Self::CompoundCommand, Self::Error>
    {
        (**self).case_command(fragments, redirects)
    }

    fn compound_command_as_pipeable(&mut self,
                                    cmd: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        (**self).compound_command_as_pipeable(cmd)
    }

    fn function_declaration(&mut self,
                            name: String,
                            post_name_comments: Vec<Newline>,
                            body: Self::CompoundCommand)
        -> ParseResult<Self::PipeableCommand, Self::Error>
    {
        (**self).function_declaration(name, post_name_comments, body)
    }

    fn comments(&mut self,
                comments: Vec<Newline>)
        -> ParseResult<(), Self::Error>
    {
        (**self).comments(comments)
    }

    fn word(&mut self,
            kind: ComplexWordKind<Self::Command>)
        -> ParseResult<Self::Word, Self::Error>
    {
        (**self).word(kind)
    }

    fn redirect(&mut self,
                kind: RedirectKind<Self::Word>)
        -> ParseResult<Self::Redirect, Self::Error>
    {
        (**self).redirect(kind)
    }
}

struct Coalesce<I: Iterator, F> {
    iter: I,
    cur: Option<I::Item>,
    func: F,
}

impl<I: Iterator, F> Coalesce<I, F> {
    fn new(iter: I, func: F) -> Self {
        Coalesce {
            iter: iter,
            cur: None,
            func: func,
        }
    }
}

type CoalesceResult<T> = ::std::result::Result<T, (T, T)>;
impl<I, F> Iterator for Coalesce<I, F>
    where I: Iterator,
          F: FnMut(I::Item, I::Item) -> CoalesceResult<I::Item>
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.cur.take().or_else(|| self.iter.next());
        let (mut left, mut right) = match (cur, self.iter.next()) {
            (Some(l), Some(r)) => (l, r),
            (Some(l), None) |
            (None, Some(l)) => return Some(l),
            (None, None) => return None,
        };

        loop {
            match (self.func)(left, right) {
                Ok(combined) => match self.iter.next() {
                    Some(next) => {
                        left = combined;
                        right = next;
                    },
                    None => return Some(combined),
                },

                Err((left, right)) => {
                    debug_assert!(self.cur.is_none());
                    self.cur = Some(right);
                    return Some(left);
                },
            }
        }
    }
}

fn compress<C>(word: ComplexWordKind<C>) -> ComplexWordKind<C> {
    use self::ComplexWordKind::*;
    use self::SimpleWordKind::*;
    use self::WordKind::*;

    fn coalesce_simple<C>(a: SimpleWordKind<C>, b: SimpleWordKind<C>)
        -> CoalesceResult<SimpleWordKind<C>>
    {
        match (a, b) {
            (Literal(mut a), Literal(b)) => {
                a.push_str(&b);
                Ok(Literal(a))
            },
            (a, b) => Err((a, b)),
        }
    }

    fn coalesce_word<C>(a: WordKind<C>, b: WordKind<C>) -> CoalesceResult<WordKind<C>> {
        match (a, b) {
            (Simple(a), Simple(b)) => coalesce_simple(a, b).map(Simple)
                                                           .map_err(|(a, b)| (Simple(a), Simple(b))),
            (SingleQuoted(mut a), SingleQuoted(b)) => {
                a.push_str(&b);
                Ok(SingleQuoted(a))
            },
            (DoubleQuoted(a), DoubleQuoted(b)) => {
                let quoted = Coalesce::new(a.into_iter().chain(b), coalesce_simple).collect();
                Ok(DoubleQuoted(quoted))
            },
            (a, b) => Err((a, b)),
        }
    }

    match word {
        Single(s) => Single(match s {
            s@Simple(_) |
            s@SingleQuoted(_) => s,
            DoubleQuoted(v) => DoubleQuoted(Coalesce::new(v.into_iter(), coalesce_simple).collect()),
        }),
        Concat(v) => {
            let mut body: Vec<_> = Coalesce::new(v.into_iter(), coalesce_word).collect();
            if body.len() == 1 {
                Single(body.pop().unwrap())
            } else {
                Concat(body)
            }
        }
    }
}
