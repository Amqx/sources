use super::*;
use aidoku::imports::std::current_date;
use jiff::{Timestamp, civil::Weekday, tz};

pub struct DayOfWeek(Weekday);

impl DayOfWeek {
    pub const fn as_id(&self) -> &'static str {
        match self.0 {
            Weekday::Monday => "0",
            Weekday::Tuesday => "1",
            Weekday::Wednesday => "2",
            Weekday::Thursday => "3",
            Weekday::Friday => "4",
            Weekday::Saturday => "5",
            Weekday::Sunday => "6",
        }
    }

    pub const fn as_name(&self) -> &'static str {
        match self.0 {
            Weekday::Monday => "週一",
            Weekday::Tuesday => "週二",
            Weekday::Wednesday => "週三",
            Weekday::Thursday => "週四",
            Weekday::Friday => "週五",
            Weekday::Saturday => "週六",
            Weekday::Sunday => "週日",
        }
    }

    pub fn today() -> Result<Self> {
        let now = current_date();
        let day_of_week = Timestamp::from_second(now)
            .map_err(|_| error!("Invalid timestamp: `{now}`"))?
            .to_zoned(tz::TimeZone::fixed(tz::offset(8)))
            .weekday();
        Ok(Self(day_of_week))
    }
}
