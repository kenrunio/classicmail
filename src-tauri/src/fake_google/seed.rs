//! Shared helpers for deterministic demo data: a small PRNG, people, companies and time anchors.

/// Deterministic xorshift PRNG so every run seeds identical data for a given e-mail.
pub struct Rng(u64);

impl Rng {
    pub fn for_key(key: &str) -> Self {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in key.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        Rng(h | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

pub fn display_name_for(email: &str) -> String {
    match email.split('@').next().unwrap_or("") {
        "kenny" => "Kenny Green".into(),
        "info" => "Duvz Software".into(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => email.into(),
            }
        }
    }
}

/// A demo person: (first, last, company, domain, title, phone).
pub struct Person {
    pub first: &'static str,
    pub last: &'static str,
    pub company: &'static str,
    pub domain: &'static str,
    pub title: &'static str,
    pub phone: &'static str,
}

impl Person {
    pub fn name(&self) -> String {
        format!("{} {}", self.first, self.last)
    }
    pub fn email(&self) -> String {
        format!("{}.{}@{}", self.first.to_lowercase(), self.last.to_lowercase(), self.domain)
    }
}

/// Fictional contacts used across mail, calendar, contacts and tasks so the demo hangs together.
pub const PEOPLE: &[Person] = &[
    Person {
        first: "Sarah",
        last: "Cohen",
        company: "Harbor Capital",
        domain: "harborcap.example",
        title: "Portfolio Manager",
        phone: "+1 212 555 0141",
    },
    Person {
        first: "David",
        last: "Levi",
        company: "Meridian Partners",
        domain: "meridian.example",
        title: "Analyst",
        phone: "+972 3 555 0192",
    },
    Person {
        first: "Michael",
        last: "Brennan",
        company: "Northgate Advisors",
        domain: "northgate.example",
        title: "Managing Director",
        phone: "+44 20 5550 1188",
    },
    Person {
        first: "Yael",
        last: "Friedman",
        company: "Tamar Investments",
        domain: "tamarinv.example",
        title: "CIO",
        phone: "+972 52 555 0113",
    },
    Person {
        first: "James",
        last: "Whitaker",
        company: "Bluewater Fund",
        domain: "bluewater.example",
        title: "Senior Analyst",
        phone: "+1 617 555 0175",
    },
    Person {
        first: "Noa",
        last: "Shapiro",
        company: "Orion Technologies",
        domain: "oriontech.example",
        title: "CFO",
        phone: "+972 9 555 0107",
    },
    Person {
        first: "Daniel",
        last: "Mizrahi",
        company: "Orion Technologies",
        domain: "oriontech.example",
        title: "Head of IR",
        phone: "+972 9 555 0108",
    },
    Person {
        first: "Emily",
        last: "Carter",
        company: "Summit Securities",
        domain: "summitsec.example",
        title: "Equity Sales",
        phone: "+1 646 555 0130",
    },
    Person { first: "Oren", last: "Katz", company: "Corvina Medical", domain: "corvina.example", title: "CEO", phone: "+972 4 555 0166" },
    Person {
        first: "Rachel",
        last: "Adler",
        company: "Corvina Medical",
        domain: "corvina.example",
        title: "VP Finance",
        phone: "+972 4 555 0167",
    },
    Person {
        first: "Thomas",
        last: "Meyer",
        company: "Rhein Asset Management",
        domain: "rhein-am.example",
        title: "Fund Manager",
        phone: "+49 69 5550 2231",
    },
    Person {
        first: "Lior",
        last: "Ben-David",
        company: "Juniper Ventures",
        domain: "junipervc.example",
        title: "Partner",
        phone: "+972 54 555 0199",
    },
    Person {
        first: "Anna",
        last: "Kowalski",
        company: "Vistula Capital",
        domain: "vistula.example",
        title: "Analyst",
        phone: "+48 22 555 0144",
    },
    Person {
        first: "Robert",
        last: "Hughes",
        company: "Eastbridge Research",
        domain: "eastbridge.example",
        title: "Research Director",
        phone: "+1 312 555 0110",
    },
    Person {
        first: "Tamar",
        last: "Peretz",
        company: "Sunfield Energy",
        domain: "sunfield.example",
        title: "Investor Relations",
        phone: "+972 8 555 0124",
    },
    Person {
        first: "Chris",
        last: "Donovan",
        company: "Atlas Conferences",
        domain: "atlasconf.example",
        title: "Event Manager",
        phone: "+1 305 555 0152",
    },
    Person {
        first: "Maya",
        last: "Goldberg",
        company: "Duvz Software",
        domain: "duvz.demo",
        title: "IR Associate",
        phone: "+972 3 555 0100",
    },
    Person {
        first: "Tom",
        last: "Bergman",
        company: "Birchwood Partners",
        domain: "birchwood.example",
        title: "Partner",
        phone: "+972 3 555 0177",
    },
    Person {
        first: "Sophie",
        last: "Laurent",
        company: "Seine Gestion",
        domain: "seinegestion.example",
        title: "Analyste",
        phone: "+33 1 5550 4411",
    },
    Person {
        first: "Avi",
        last: "Rosen",
        company: "Jerusalem Equity",
        domain: "jlmequity.example",
        title: "Trader",
        phone: "+972 2 555 0185",
    },
];

/// "Now" for seeding: the app's clock (real time in the demo, so today / yesterday look right;
/// a fixed date in the unit tests, so the seed is the same on every run).
pub fn now_ms() -> i64 {
    crate::clock::now_ms()
}

pub const DAY_MS: i64 = 86_400_000;
pub const HOUR_MS: i64 = 3_600_000;
