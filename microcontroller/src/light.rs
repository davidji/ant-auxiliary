use crate::{
    proto::{ 
        LightRequest, 
        LightRequest_, 
        LightResponse, 
        Response, 
        Response_::Peripheral as ResponsePeripheral,
    },
    codec::{ ResponseSender},
};

use defmt::{ info, warn };
use embedded_hal::pwm::SetDutyCycle;


pub struct Light<PWM: SetDutyCycle> {
    pwm: PWM,
    responses: ResponseSender,
    curent_duty: f32,
}

impl <PWM: SetDutyCycle> Light<PWM> {
    pub fn new(pwm: PWM, responses: ResponseSender) -> Self {
        Light {
            pwm,
            responses,
            curent_duty: 0.0,
        }
    }

    pub async fn process(&mut self, request: LightRequest) {
       match request {
            LightRequest { command: Some(LightRequest_::Command::Set(set)) } => {
                info!("Light set duty {}", set.duty);
                self.pwm.set_duty_cycle((set.duty*self.pwm.max_duty_cycle() as f32) as u16).unwrap();
                self.curent_duty = set.duty;
            },
            LightRequest { command: Some(LightRequest_::Command::Get(_)) } => { },
            LightRequest { command: _ } => {
                warn!("Unknown command for Light");
            }
        }

        let response = self.response();
        self.responses.send(response).await.unwrap();
    }

    fn response(&mut self) -> Response {
        Response { peripheral: Some(ResponsePeripheral::Light(LightResponse {
            duty: self.curent_duty,
        })) }
    }
}
