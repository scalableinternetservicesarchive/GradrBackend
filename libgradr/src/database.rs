// Abstraction of the database.
// For the purposes of the backend, the only necessary
// actions are the following:
//
// 1. Put a pending build into the database.
// 2. Grab a pending build from the database, which
//    will immediately undergo being built.
// 3. Put test results into a database, given what
//    build was pending

extern crate postgres;
#[phase(plugin)]
extern crate pg_typeprovider;

use builder::BuildResult;

use self::EntryStatus::{Pending, InProgress, Done};

pub trait DatabaseEntry<A> : Send {
    fn get_base(&self) -> A;
}

impl<A : Send + Clone> DatabaseEntry<A> for A {
    fn get_base(&self) -> A { self.clone() }
}

/// Type A is some key
pub trait Database<A, B : DatabaseEntry<A>> : Sync + Send {
    fn add_pending(&self, entry: A);

    /// Optionally gets a pending build from the database.
    /// If `Some` is returned, it will not be returned again.
    /// If `None` is returned, it is expected that the caller will sleep.
    fn get_pending(&self) -> Option<B>;

    fn add_test_results(&self, entry: B, results: BuildResult);
}

pub enum EntryStatus {
    Pending,
    InProgress,
    Done
}

impl EntryStatus {
    pub fn to_int(&self) -> i32 {
        match *self {
            Pending => 0,
            InProgress => 1,
            Done => 2
        }
    }
}

pub mod postgres_db {
    extern crate pg_typeprovider;

    use self::pg_typeprovider::util::Joinable;

    use std::sync::Mutex;

    use super::postgres::{Connection, SslMode, ToSql};

    use builder::BuildResult;
    use super::EntryStatus::{Pending, InProgress, Done};
    use super::{Database, DatabaseEntry};

    pg_table!(builds)

    pub struct PostgresDatabase {
        db: Mutex<Connection>
    }

    impl PostgresDatabase {
        pub fn new(loc: &str) -> Option<PostgresDatabase> {
            Connection::connect(loc, &SslMode::None).ok().map(|db| {
                PostgresDatabase {
                    db: Mutex::new(db)
                }
            })
        }

        pub fn new_testing() -> Option<PostgresDatabase> {
            let retval = PostgresDatabase::new(
                "postgres://jroesch@localhost/gradr-test");
            match retval {
                Some(ref db) => {
                    db.with_connection(|conn| {
                        conn.execute(
                            "DELETE FROM users", &[]).unwrap();
                        conn.execute(
                            "DELETE FROM builds", &[]).unwrap();
                    })
                },
                None => ()
            };
            retval
        }

        pub fn with_connection<A>(&self, f: |&Connection| -> A) -> A {
            f(&*self.db.lock())
        }
    }

    impl DatabaseEntry<BuildInsert> for Build {
        fn get_base(&self) -> BuildInsert {
            BuildInsert {
                status: self.status,
                clone_url: self.clone_url.clone(),
                branch: self.branch.clone(),
                results: self.results.clone()
            }
        }
    }

    fn get_one_build(conn: &Connection) -> Option<Build> {
        BuildSearch::new()
            .where_status((&Pending).to_int())
            .search(conn, Some(1)).pop()
    }

    // returns true if it was able to lock it, else false
    fn try_lock_build(conn: &Connection, b: &Build) -> bool {
        BuildUpdate::new()
            .status_to((&InProgress).to_int())
            .where_id(b.id)
            .where_status((&Pending).to_int())
            .update(conn) == 1
    }

    impl Database<BuildInsert, Build> for PostgresDatabase {
        fn add_pending(&self, entry: BuildInsert) {
            self.with_connection(|conn| entry.insert(conn));
        }

        fn get_pending(&self) -> Option<Build> {
            self.with_connection(|conn| {
                loop {
                    match get_one_build(conn) {
                        Some(b) => {
                            if try_lock_build(conn, &b) {
                                return Some(b);
                            }
                        },
                        None => { return None; }
                    }
                }
            })
        }

        fn add_test_results(&self, entry: Build, results: BuildResult) {
            self.with_connection(|conn| {
                let num_updated = 
                    BuildUpdate::new()
                    .status_to((&Done).to_int())
                    .results_to(results.to_string())
                    .where_id(entry.id)
                    .update(conn);
                assert_eq!(num_updated, 1);
            });
        }
    }
}