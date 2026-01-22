type Error = Box<dyn std::error::Error>;  
type Result<T> = std::result::Result<T, Error>;  
 
#[derive(Copy, Clone, Debug, PartialEq, Eq)] 
pub enum Command { 
    SET = 0, 
    GET = 1, 
    DEL = 2, 
} 
 
impl Command { 
    pub fn from_byte(byte: u8) -> Result<Command> { 
        match byte { 
            0 => Ok(Command::SET), 
            1 => Ok(Command::GET), 
            2 => Ok(Command::DEL), 
            _ => Err(Box::from("Unknown command")), 
        } 
    } 
} 

