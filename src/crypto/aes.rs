#![allow(dead_code)]
use aes::Aes128;
use aes::cipher::{BlockCipher, NewBlockCipher};
use generic_array::typenum::{U16};
use generic_array::GenericArray;

type CipherBlock = fn (&mut GenericArray<u8, U16>);

struct HexEncoder {}
pub struct EncryptorAES {
    key: [u8; 16],
    iv: Option<[u8; 16]>,
}

trait Block {}

impl dyn Block {
    fn get_chunks(data: &[u8]) -> Vec<GenericArray<u8, U16>> {
        let mut blocks = Vec::default();
        for chunk in data.chunks(16) {
            let mut padding_size = 0u8;
            if chunk.len() < 16 {
                padding_size = (16 - chunk.len()) as u8;
            }
            let mut temp: [u8; 16] = [padding_size; 16];
            chunk
                .iter()
                .enumerate()
                .for_each(|(index, c)| temp[index] = *c);

            //println!("chunk {:?} {:?}", chunk, temp);
            let generic_data: GenericArray<u8, U16> = GenericArray::from(temp);
            blocks.push(generic_data);
        }

        blocks
    }

    fn xor_blocks(res_block: &mut GenericArray<u8, U16>, other_block: &GenericArray<u8, U16>){
        let other_size = other_block.len();
        for (i, b) in res_block.iter_mut().enumerate() {
            *b = *b ^ other_block[i % other_size];
        }
    }
}

impl EncryptorAES {

    pub fn new(key: [u8; 16]) -> Self {
        EncryptorAES {
            key,
            iv: None,
        }
    }

    pub fn new_cbc(key: [u8; 16], iv: [u8; 16]) -> Self {
        EncryptorAES {
            key,
            iv: Some(iv),
        }
    }

    pub fn encrypt(&self, data: &[u8]) -> Vec<u8> {
        let gen_key: GenericArray<u8, U16> = GenericArray::from(self.key);
        let cipher = Aes128::new(&gen_key);
        let mut blocks = Block::get_chunks(data);
        let mut prev_block: Option<&GenericArray<u8, U16>> = None;
        
        if let Some(iv) = self.iv {
            // XOR if CBC
            return blocks.iter_mut().flat_map(|block|{
                match prev_block{
                    Some(pblock) => {
                        Block::xor_blocks(block, pblock);
                    },
                    None => {
                        Block::xor_blocks(block, &GenericArray::from(iv));
                    },
                }
                cipher.encrypt_block(block);
                prev_block = Some(block);
                *block
            }).collect();
        } else {
            return blocks.iter_mut().flat_map(|block|{
                cipher.encrypt_block(block);
                *block
            }).collect();
        }
    }

    pub fn decrypt(&self, data: Vec<u8>) -> Vec<u8> {
        let gen_key: GenericArray<u8, U16> = GenericArray::from(self.key);
        let cipher = Aes128::new(&gen_key);
        let mut blocks = Block::get_chunks(data.as_slice());
        let mut prev_block: Option<GenericArray<u8, U16>> = None;

        let decrypted_with_padding: Vec<u8>;

        if let Some(iv) = self.iv {
            // XOR if CBC
            decrypted_with_padding = blocks.iter_mut().flat_map(|block|{
                let prev_block_enc_copy = block.iter().map(|b| *b).collect::<GenericArray<u8, U16>>();
                cipher.decrypt_block(block);
                match prev_block{
                    Some(pblock) => {
                        Block::xor_blocks(block, &pblock);
                    },
                    None => {
                        Block::xor_blocks(block, &GenericArray::from(iv));
                    },
                }
                prev_block = Some(prev_block_enc_copy);
                *block
            }).collect();
        } else {
            decrypted_with_padding = blocks.iter_mut().flat_map(|block|{
                cipher.decrypt_block(block);
                *block
            }).collect();
        }

        // remove padding; last byte is the padding size;
        // all padding bytes are the padding size
        if let Some(padding_size) = decrypted_with_padding.last(){
            let mut padding_ok = true;
            if (*padding_size as usize) < decrypted_with_padding.len() {
                let start_padding = decrypted_with_padding.len() - (*padding_size as usize);
                for b in decrypted_with_padding[start_padding..].iter(){
                    if b != padding_size {
                        println!("{} == {}", b, padding_size);
                        padding_ok = false;
                        break;
                    }
                }
                if padding_ok {
                    return decrypted_with_padding[0..start_padding].to_vec();
                }
            }
        }

        return decrypted_with_padding;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn aes_works() {
        const INPUT: &str = "Hello world12345";
        let encryptor = EncryptorAES::new(hex!("01020304050607080910111213141516"));
        let encrypted_text = encryptor.encrypt(INPUT.as_bytes());
        let decrypted_text = encryptor.decrypt(encrypted_text.to_owned());

        println!("ENCRYPTED TEXT: {:?}", encrypted_text);
        println!("DECRYPTED TEXT: {:?}", decrypted_text);
        assert_eq!(INPUT.as_bytes(), decrypted_text);
    }

    #[test]
    fn aes_cbc_works() {
        const INPUT: &str = "Hello world12345";
        let encryptor = EncryptorAES::new_cbc(hex!("01020304050607080910111213141516"), hex!("01020304050607080910111213141516"));
        let encrypted_text = encryptor.encrypt(INPUT.as_bytes());
        let decrypted_text = encryptor.decrypt(encrypted_text.to_owned());

        println!("ENCRYPTED TEXT: {:?}", encrypted_text);
        println!("DECRYPTED TEXT: {:?}", decrypted_text);
        assert_eq!(INPUT.as_bytes(), decrypted_text);
    }
}